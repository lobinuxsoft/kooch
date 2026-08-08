// inti_pbr.wgsl — the shading model. Cook-Torrance driven by the light
// components, shared verbatim by every path that shades a surface.
//
// WGSL has no #include, so this chunk is CONCATENATED in Rust ahead of
// each consumer (the same trick `compose_material_shader` already uses
// for `visibility_buffer_resolve.wgsl`). `{{INTI_GROUP}}` is replaced
// with the consumer's free bind-group index before module creation:
// the R64 two-pass path has 0..4 taken, the R32 compute path 0..3.
//
// # Why one file
//
// Two shading paths ship (R64 two-pass fragment, R32 compute deferred)
// and a third arrives with shadows. A BRDF copied per path diverges
// silently: the difference only ever shows up as "the fallback GPU
// looks a bit off", which nobody owns. The bindings differ per path;
// the maths does not, so the maths lives here once.
//
// # Provenance — read before editing a term
//
// The terms are ported from Bevy 0.19's `pbr_lighting.wgsl` (itself
// Filament's model), read from source rather than reconstructed. Three
// of their fixes are baked in from the start:
//
// - **0.18, bevy#22454** — their environment-map light used a
//   roughness-dependent Fresnel that made every material read as wet.
//   `inti_fresnel` is the plain Schlick term they moved back to.
// - **0.18, bevy#22372** — their point/area specular used the base
//   roughness where the sphere-light solid angle demanded the widened
//   `a_prime` (Karis 2013), so highlights stayed sharp and far too
//   bright with distance. Our punctual lights have no radius, so there
//   is no `a_prime` to get wrong yet — but the day `PointLight` grows
//   one, the specular term must widen with it. Recorded before we fall
//   in.
// - **0.19, Solari** — the specular and diffuse lobes each claiming the
//   full incoming light. `inti_shade` weights the diffuse by `(1 - F)`.
//   Note this **diverges from Bevy's own forward path**, which still
//   adds the two lobes unweighted the way Filament does: we take the
//   newer, physically-layered form because a mirror is where the
//   difference is visible and a mirror is not an edge case.
//
// `D_GGX` and `V_SmithGGXCorrelated` take **linear** roughness
// (`a = perceptual²`), `F_AB` takes **perceptual**. Feeding the wrong
// one to either is the classic way to get a BRDF that looks plausible
// and is wrong everywhere.

const INTI_PI: f32 = 3.14159265359;

const INTI_KIND_DIRECTIONAL: u32 = 0u;
const INTI_KIND_POINT: u32 = 1u;
const INTI_KIND_SPOT: u32 = 2u;

// Filament's f32 floor on perceptual roughness. Below it the GGX lobe
// is a delta function: a punctual light either misses it entirely or
// lands one blinding pixel on it.
const INTI_MIN_PERCEPTUAL_ROUGHNESS: f32 = 0.089;

// Mirror of `kooch_lighting::GpuLight` (64 B, `#[repr(C)]`). Field
// order and padding are load-bearing: a mismatch reads a light's range
// as its intensity and there is no compiler on either side of that
// boundary. `gpu_light.rs` holds the test that pins the size.
struct IntiLight {
    color: vec3<f32>,
    // Photometric, straight off the component so the Inspector's number
    // and the buffer's number are the same number: lux for directional,
    // lumens for punctual. The radiometric conversion happens below.
    intensity: f32,
    position: vec3<f32>,
    range: f32,
    // World-space, normalised, pointing where the light points (the
    // entity's -Z). Unused by point lights.
    direction: vec3<f32>,
    kind: u32,
    // Cone falloff as a multiply-add: `saturate(cd * scale + offset)`,
    // one MAD per light per fragment instead of a subtract and a
    // divide. Bevy packs it the same way. The authored half-angles are
    // recoverable: `cos_outer = -offset / scale`,
    // `cos_inner = cos_outer + 1 / scale`.
    spot_scale: f32,
    spot_offset: f32,
    // Per-light opt-ins, one bit each. Took the first of the two pad
    // words rather than growing the record: a scene with fifty lights
    // should not pay fifty screen-space marches per pixel, and the
    // switch that prevents it costs nothing it was not already
    // reserving.
    flags: u32,
    _pad0: f32,
}

// Bit 0 of `IntiLight.flags` — this light marches for contact shadows.
// Mirrors `GpuLight::FLAG_CONTACT_SHADOWS`.
const INTI_LIGHT_CONTACT_SHADOWS: u32 = 1u;

// Per-frame lighting constants. `camera_position` lives here rather
// than in the shared camera UBO because that UBO is pinned at 64 B by
// two bind-group layouts, and widening it would ripple through paths
// this issue has no business touching.
// One cascade: where it lives in the atlas and how to get there.
struct IntiCascade {
    // Light-space clip-from-world.
    view_proj: mat4x4<f32>,
    // Maps this cascade's [0,1] shadow uv into the atlas: xy scale,
    // zw bias. One fma instead of a branch on which quadrant.
    uv_scale_bias: vec4<f32>,
    // View-space depth past which the next cascade takes over.
    far_depth: f32,
    // World units per shadow texel, for the filter radius and the
    // penumbra estimate. A fixed radius in texels is a different
    // distance in every cascade.
    texel_world_size: f32,
    // World units the [0,1] depth range spans. Turns a difference
    // between two stored depths back into a distance in metres, which
    // is what the penumbra is proportional to. Without it the shader
    // has a ratio and no scale.
    depth_extent: f32,
    _pad0: f32,
}

struct IntiFrame {
    ambient_sky: vec3<f32>,
    light_count: u32,
    ambient_ground: vec3<f32>,
    // 1 / (2^EV100 * 1.2). Fixed until auto exposure lands (#254).
    // Without it a 10 000 lux sun clips every channel to white and the
    // whole model reads as "broken" rather than "unexposed".
    exposure: f32,
    camera_position: vec3<f32>,
    ambient_intensity: f32,
    // Unit vector down the view axis. Only the shadow cascades need it,
    // and they need it to be the axis rather than the radial direction —
    // see `inti_shade`.
    camera_forward: vec3<f32>,
    _pad_forward: f32,
    // The four cascades. Fixed-size because the count is baked into the
    // atlas layout: changing it is a texture change, not a loop bound.
    cascades: array<IntiCascade, 4>,
    // 0 when no directional light casts, or the atlas has not been
    // rendered. The dummy 1x1 atlas bound in that case would return
    // "fully lit" anyway; the flag skips the work.
    shadows_enabled: u32,
    // Fraction of a split distance over which one cascade fades into the
    // next. Without it the boundary is a visible line where texel
    // density and filter radius change at once.
    cascade_blend: f32,
    // Tangent of the sun's angular RADIUS: how much wider a shadow gets
    // per metre of gap between blocker and receiver. An angle, not a
    // width, because that is what a light infinitely far away has.
    sun_softness: f32,
    _pad_frame: f32,
}

@group({{INTI_GROUP}}) @binding(0) var<uniform> inti: IntiFrame;
// Always at least one element: wgpu rejects a zero-sized storage
// binding, so an unlit scene binds a one-element buffer with
// `light_count == 0` rather than needing a second pipeline.
@group({{INTI_GROUP}}) @binding(1) var<storage, read> inti_lights: array<IntiLight>;

// Cascaded shadow maps (#476). In Inti's group rather than a seventh of
// their own: the bind-group budget is fully spent, and a shadow map
// without its light is not a thing any shader wants anyway.
//
// A comparison sampler, so the hardware does the depth test and returns
// a filtered occlusion fraction rather than a depth to compare by hand —
// bilinear PCF for free, on the texture unit.
@group({{INTI_GROUP}}) @binding(2) var inti_shadow_atlas: texture_depth_2d;
@group({{INTI_GROUP}}) @binding(3) var inti_shadow_sampler: sampler_comparison;
// A second sampler on the SAME texture, non-comparison, for the blocker
// search: it needs the stored depth, and a comparison sampler only ever
// answers "nearer or not". Bevy binds exactly this pair
// (`directional_shadow_textures_linear_sampler`).
@group({{INTI_GROUP}}) @binding(4) var inti_shadow_point_sampler: sampler;

// GGX / Trowbridge-Reitz, in Filament's reassociated form. The naïve
// `a2 / (π·((NdotH²)(a2-1)+1)²)` loses catastrophic precision in f32 at
// low roughness — the highlight breaks into blocks. `a` is linear.
fn inti_d_ggx(a: f32, n_dot_h: f32) -> f32 {
    let one_minus_n_dot_h_sq = 1.0 - n_dot_h * n_dot_h;
    let x = n_dot_h * a;
    let k = a / (one_minus_n_dot_h_sq + x * x);
    return k * k * (1.0 / INTI_PI);
}

// Height-correlated Smith visibility (Heitz 2014). Returns the combined
// G / (4·NoV·NoL), so the specular term must NOT divide again — the
// double divide is why hand-rolled BRDFs come out black at grazing
// angles. `a` is linear.
fn inti_v_smith_correlated(a: f32, n_dot_v: f32, n_dot_l: f32) -> f32 {
    let a2 = a * a;
    let lambda_v = n_dot_l * sqrt((n_dot_v - a2 * n_dot_v) * n_dot_v + a2);
    let lambda_l = n_dot_v * sqrt((n_dot_l - a2 * n_dot_l) * n_dot_l + a2);
    return 0.5 / max(lambda_v + lambda_l, 1e-4);
}

fn inti_f_schlick_scalar(f0: f32, f90: f32, v_dot_h: f32) -> f32 {
    return f0 + (f90 - f0) * pow(saturate(1.0 - v_dot_h), 5.0);
}

// f90 derived from f0 rather than assumed to be 1.0. A near-black
// dielectric with f90 = 1 grows a white rim at grazing angles that no
// real material has; the 50·0.33 scale is Filament's fit.
fn inti_fresnel(f0: vec3<f32>, v_dot_h: f32) -> vec3<f32> {
    let f90 = saturate(dot(f0, vec3<f32>(50.0 * 0.33)));
    return f0 + (vec3<f32>(f90) - f0) * pow(saturate(1.0 - v_dot_h), 5.0);
}

// Analytic fit to the split-sum DFG integral (Karis / Lazarov), the
// same polynomial Bevy falls back to without a DFG LUT. Feeds the
// multiscatter compensation below. Takes PERCEPTUAL roughness.
fn inti_f_ab(perceptual_roughness: f32, n_dot_v: f32) -> vec2<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = perceptual_roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    return max(vec2<f32>(-1.04, 1.04) * a004 + r.zw, vec2<f32>(0.00005));
}

// Single-scattering GGX loses energy on rough metals — they go grey
// instead of staying bright. Compensates by the fraction the split-sum
// integral says went missing.
fn inti_specular_multiscatter(
    single_scatter: vec3<f32>,
    f0: vec3<f32>,
    f_ab: vec2<f32>,
) -> vec3<f32> {
    return single_scatter * (1.0 + f0 * (1.0 / (f_ab.x + f_ab.y) - 1.0));
}

// Burley / Disney diffuse. Lambert is flat; this one brightens the
// grazing edge on rough surfaces the way cloth and unfinished wood
// actually do. `a` unused — Burley takes perceptual roughness.
fn inti_fd_burley(
    perceptual_roughness: f32,
    n_dot_v: f32,
    n_dot_l: f32,
    l_dot_h: f32,
) -> f32 {
    let f90 = 0.5 + 2.0 * perceptual_roughness * l_dot_h * l_dot_h;
    let light_scatter = inti_f_schlick_scalar(1.0, f90, n_dot_l);
    let view_scatter = inti_f_schlick_scalar(1.0, f90, n_dot_v);
    return light_scatter * view_scatter * (1.0 / INTI_PI);
}

// Inverse-square with a smooth window at `range`. A hard cutoff pops
// the moment a light or the geometry moves; the squared smoothstep
// reaches exactly zero at the range the editor gizmo draws, so the
// wire sphere in the viewport is the truth and not an approximation
// of it.
fn inti_distance_attenuation(distance_sq: f32, range: f32) -> f32 {
    let inv_range_sq = 1.0 / max(range * range, 1e-4);
    let factor = distance_sq * inv_range_sq;
    let window = saturate(1.0 - factor * factor);
    return (window * window) / max(distance_sq, 1e-4);
}

// Per-light irradiance and direction, resolved by kind.
struct IntiSample {
    // Unit vector from the surface toward the light.
    to_light: vec3<f32>,
    // Radiometric irradiance arriving perpendicular to the surface,
    // pre-exposure. Zero when the fragment is out of the light's reach.
    irradiance: vec3<f32>,
}

fn inti_sample_light(light: IntiLight, world_position: vec3<f32>) -> IntiSample {
    var out: IntiSample;
    if (light.kind == INTI_KIND_DIRECTIONAL) {
        // Infinitely far away: no falloff, and illuminance in lux IS
        // the perpendicular irradiance.
        out.to_light = -light.direction;
        out.irradiance = light.color * light.intensity;
        return out;
    }

    let offset = light.position - world_position;
    let distance_sq = dot(offset, offset);
    out.to_light = offset * inverseSqrt(max(distance_sq, 1e-8));

    // Luminous flux (lm) → luminous intensity (cd) over the full
    // sphere. Spot lights divide by 4π too, NOT by the cone's solid
    // angle: narrowing a cone should aim a light, not brighten it.
    // Unity and Bevy both chose this; the alternative is an artist
    // widening a cone and watching the scene go dark.
    let intensity = light.intensity / (4.0 * INTI_PI);
    var attenuation = inti_distance_attenuation(distance_sq, light.range);
    if (light.kind == INTI_KIND_SPOT) {
        let cd = dot(-light.direction, out.to_light);
        let cone = saturate(cd * light.spot_scale + light.spot_offset);
        attenuation *= cone * cone;
    }
    out.irradiance = light.color * intensity * attenuation;
    return out;
}

// Hemisphere ambient — the cheapest thing that keeps a metal from
// reading as a black hole with no environment map bound. Stands in for
// real IBL (#450).
//
// ⚠️ `n.y` is world up, which stops meaning anything on the far side of
// a planet. A known limit of the placeholder, not an oversight: the
// replacement is a probe, not a smarter up vector.
fn inti_ambient(
    n: vec3<f32>,
    diffuse_color: vec3<f32>,
    f0: vec3<f32>,
    f_ab: vec2<f32>,
) -> vec3<f32> {
    let t = n.y * 0.5 + 0.5;
    let sky = mix(inti.ambient_ground, inti.ambient_sky, t) * inti.ambient_intensity;
    // Split-sum specular against a constant environment: F0·f_ab.x +
    // f_ab.y is exactly what the DFG term evaluates to when the
    // prefiltered radiance is uniform, so this is the correct answer
    // for this environment rather than a fudge.
    let specular = f0 * f_ab.x + f_ab.y;
    // The diffuse layer receives what the specular layer did not
    // reflect — weighted by the SAME term, which is the whole point.
    // Weighting it by a Schlick evaluated at N·V instead (the obvious
    // thing, and what this did first) mixes two approximations of one
    // quantity, so the two halves stop summing to one and the ambient
    // term quietly stops conserving energy.
    return sky * (diffuse_color * (vec3<f32>(1.0) - specular) + specular);
}

// ── Shadows (#476) ──────────────────────────────────────────────────
//
// PCSS: contact-hardening soft shadows. Three steps, and the middle one
// is why it looks like a shadow rather than like a blurred stencil.
//
//   1. Blocker search — average depth of whatever is between this point
//      and the light, over a small disc.
//   2. Penumbra estimate — how wide the shadow edge should be, from the
//      distance between the receiver and its blockers. Close to the
//      occluder it collapses to nothing; far away it spreads.
//   3. Filter — PCF at that width.
//
// A fixed-radius PCF gives every edge the same softness, which reads as
// out of focus. The width varying with occluder distance is the whole
// effect.




// Which cascade covers `view_depth`, and how far into the blend band it
// is. `x` is the index, `y` is 0 inside the cascade and rises to 1 at
// its far edge.
fn inti_pick_cascade(view_depth: f32) -> vec2<f32> {
    for (var i = 0u; i < 4u; i = i + 1u) {
        let far = inti.cascades[i].far_depth;
        if (view_depth < far) {
            let band = far * inti.cascade_blend;
            let blend = select(0.0, (view_depth - (far - band)) / max(band, 1e-4), band > 0.0);
            return vec2<f32>(f32(i), saturate(blend));
        }
    }
    // Past the last cascade there is nothing to sample. Reported as
    // index 4 so the caller returns fully lit rather than clamping to
    // the last cascade and stretching its shadow to the horizon.
    return vec2<f32>(4.0, 0.0);
}

// World position → this cascade's shadow uv and depth. `w` is 0 when the
// point falls outside the cascade at all.
fn inti_shadow_coords(cascade: IntiCascade, world_position: vec3<f32>) -> vec4<f32> {
    let clip = cascade.view_proj * vec4<f32>(world_position, 1.0);
    // Orthographic, so w is 1 and the divide is free — done anyway
    // because a perspective cascade (a spot light's, later) would need
    // it and silently producing garbage there is worse than a divide.
    let ndc = clip.xyz / clip.w;
    if (any(abs(ndc.xy) > vec2<f32>(1.0)) || ndc.z <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    // NDC to uv: x maps directly, y flips because texture space counts
    // down from the top.
    var uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    uv = uv * cascade.uv_scale_bias.xy + cascade.uv_scale_bias.zw;
    return vec4<f32>(uv, ndc.z, 1.0);
}

// The eight D3D MSAA sample positions, for the blocker search. Chosen
// for a distribution that is not too regular; Bevy uses the same set.
const INTI_BLOCKER_TAPS: array<vec2<f32>, 8> = array<vec2<f32>, 8>(
    vec2<f32>(0.125, -0.375), vec2<f32>(-0.125, 0.375),
    vec2<f32>(0.625, 0.125), vec2<f32>(-0.375, -0.625),
    vec2<f32>(-0.625, 0.625), vec2<f32>(-0.875, -0.125),
    vec2<f32>(0.375, 0.875), vec2<f32>(0.875, -0.875),
);

/// Depth pushed along the direction to the light, in world units.
///
/// Bevy 0.19's `shadow_depth_bias` default, and it does this in world
/// space rather than through the rasteriser's depth bias — which is why
/// the shadow pipeline here no longer sets one. Two biases doing the
/// same job is how a shadow ends up detached from its object while
/// still showing acne somewhere else.
const INTI_DEPTH_BIAS: f32 = 0.02;

/// How far along its own normal a surface's sample point is pushed, in
/// shadow texels. Bevy 0.19's `shadow_normal_bias` default.
///
/// A surface edge-on to the light covers many world units inside one
/// shadow texel, and no constant depth bias covers that. Moving along
/// the surface instead of pushing depth is what stops acne without
/// detaching the shadow.
const INTI_NORMAL_BIAS: f32 = 1.8;

// World units → atlas uv, for this cascade.
//
// The cascade's own scale cancels out: a quadrant is `uv_scale_bias.x`
// of the atlas and spans `texel_world_size × atlas_size ×
// uv_scale_bias.x` world units, so the ratio is the same expression in
// every quadrant. Same relation Bevy uses (`1 / (texel_size ×
// shadow_map_size)`) — a distance in metres is the one honest unit for
// a filter radius, and texels are not.
fn inti_world_to_atlas_uv(cascade: IntiCascade) -> f32 {
    let atlas = f32(textureDimensions(inti_shadow_atlas).x);
    return 1.0 / max(cascade.texel_world_size * atlas, 1e-6);
}

// Average stored depth of whatever is between this point and the light,
// and whether there was anything. Under reversed-Z an occluder is
// CLOSER to the light and therefore stored GREATER than the receiver.
fn inti_blocker_depth(
    uv: vec2<f32>,
    receiver_depth: f32,
    radius_uv: f32,
    bounds: vec4<f32>,
) -> vec2<f32> {
    var sum = vec2<f32>(0.0);
    for (var i = 0; i < 8; i = i + 1) {
        let tap = clamp(uv + INTI_BLOCKER_TAPS[i] * radius_uv, bounds.xy, bounds.zw);
        // A plain sample, not a comparison one — see the binding. The
        // level is an INTEGER on a depth texture; 0.0 is
        // `InvalidSampleLevelExactType` out of naga, which reads like a
        // sampler problem and is a literal one.
        let depth = textureSampleLevel(
            inti_shadow_atlas, inti_shadow_point_sampler, tap, 0u);
        sum += select(vec2<f32>(0.0), vec2<f32>(depth, 1.0), depth > receiver_depth);
    }
    if (sum.y == 0.0) {
        return vec2<f32>(0.0, 0.0);
    }
    return vec2<f32>(sum.x / sum.y, 1.0);
}

// Castano '13 — the filter Bevy ships as its default, ported from
// `shadow_sampling.wgsl`.
//
// Nine hardware comparison taps, each already bilinear, weighted so the
// pair covers a 5×5 Gaussian. The weights and the sub-texel `s`/`t`
// terms are what make the result smooth: the kernel is positioned
// against the texel grid rather than around the sample point, so the
// filter does not swim as the surface moves within a texel.
//
// 🔴 This replaces sixteen Poisson taps at a variable radius, which is
// what a shadow edge with visible steps and a ringed halo looks like.
// Poisson turns undersampling into noise, and noise is only the better
// trade when a temporal pass resolves it — Bevy's own comment says its
// non-temporal PCSS is "rather noisy", and there is no TAA here yet
// (#732).
//
// `scale` widens the kernel for PCSS. Bevy switches filters instead
// because Castano's size is hard-wired; scaling the offsets keeps the
// Gaussian weighting, which is the part that stops the banding.
//
// `bounds` is the quadrant this cascade occupies, as `xy` = min uv and
// `zw` = max. Every tap is clamped into it.
//
// 🔴 Without that clamp a tap near a quadrant edge reads the NEIGHBOURING
// cascade — depth from a different volume entirely, so it answers a
// question about a different part of the world. The sampler's
// clamp-to-edge cannot help: the atlas edge is four quadrants away. It
// stays quiet until the kernel widens, which is exactly what PCSS does
// where the penumbra is largest, so it would have surfaced as "soft
// shadows are wrong near cascade boundaries" and looked like a blend
// bug.
fn inti_sample_castano(uv: vec2<f32>, depth: f32, scale: f32, bounds: vec4<f32>) -> f32 {
    let map_size = vec2<f32>(textureDimensions(inti_shadow_atlas));
    let inv_map_size = 1.0 / map_size;

    let texel_uv = uv * map_size;
    var base_uv = floor(texel_uv + 0.5);
    let s = texel_uv.x + 0.5 - base_uv.x;
    let t = texel_uv.y + 0.5 - base_uv.y;
    base_uv = (base_uv - 0.5) * inv_map_size;

    let uw0 = 4.0 - 3.0 * s;
    let uw1 = 7.0;
    let uw2 = 1.0 + 3.0 * s;
    let u0 = (3.0 - 2.0 * s) / uw0 - 2.0;
    let u1 = (3.0 + s) / uw1;
    let u2 = s / uw2 + 2.0;

    let vw0 = 4.0 - 3.0 * t;
    let vw1 = 7.0;
    let vw2 = 1.0 + 3.0 * t;
    let v0 = (3.0 - 2.0 * t) / vw0 - 2.0;
    let v1 = (3.0 + t) / vw1;
    let v2 = t / vw2 + 2.0;

    let step = inv_map_size * scale;
    let us = array<f32, 3>(u0, u1, u2);
    let vs = array<f32, 3>(v0, v1, v2);
    let uw = array<f32, 3>(uw0, uw1, uw2);
    let vw = array<f32, 3>(vw0, vw1, vw2);

    var sum = 0.0;
    for (var j = 0; j < 3; j = j + 1) {
        for (var i = 0; i < 3; i = i + 1) {
            let tap = clamp(base_uv + vec2<f32>(us[i], vs[j]) * step, bounds.xy, bounds.zw);
            sum += uw[i] * vw[j] * textureSampleCompareLevel(
                inti_shadow_atlas, inti_shadow_sampler, tap, depth);
        }
    }
    return sum * (1.0 / 144.0);
}

/// Occlusion from one cascade, 1 = fully lit.
fn inti_sample_cascade(
    index: u32,
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_light: vec3<f32>,
    n_dot_l: f32,
) -> f32 {
    let cascade = inti.cascades[index];

    // Both of Bevy's offsets, in world space, and no rasteriser depth
    // bias to go with them. The normal term scales with how obliquely
    // the light hits, because that is when one texel covers the most
    // surface.
    // No slope term. Bevy scales the normal offset by the cascade's
    // texel size and nothing else, and the extra factor was mine: it
    // doubles the offset on grazing surfaces, which is exactly where a
    // shadow detaching from its object is most visible.
    let offset_position = world_position
        + normal * (cascade.texel_world_size * INTI_NORMAL_BIAS)
        + to_light * INTI_DEPTH_BIAS;

    let coords = inti_shadow_coords(cascade, offset_position);
    if (coords.w == 0.0) {
        return 1.0;
    }

    let to_uv = inti_world_to_atlas_uv(cascade);
    // This cascade's quadrant, inset by half a texel so a bilinear tap
    // on the boundary cannot fetch its neighbour either.
    let half_texel = 0.5 / f32(textureDimensions(inti_shadow_atlas).x);
    let bounds = vec4<f32>(
        cascade.uv_scale_bias.zw + half_texel,
        cascade.uv_scale_bias.zw + cascade.uv_scale_bias.xy - half_texel,
    );

    // 1. Blocker search, over a disc as wide as the softest penumbra the
    // sun could produce across this cascade's depth range.
    let search_world = max(
        inti.sun_softness * cascade.depth_extent, cascade.texel_world_size * 2.0);
    let blocker = inti_blocker_depth(coords.xy, coords.z, search_world * to_uv, bounds);
    if (blocker.y == 0.0) {
        return 1.0;
    }

    // 2. Penumbra. Reversed-Z, so the blocker is stored greater;
    // scaling by the cascade's depth extent turns the difference into
    // the gap in metres.
    //
    // ⚠️ Deliberately NOT Bevy's `(z_blocker - depth) * light_size /
    // depth`. That divide is the perspective form, and a directional
    // cascade is orthographic — under it `depth` is distance from the
    // light's near plane, so dividing by it makes the blur depend on how
    // far the receiver is from the sun rather than from its blocker, and
    // it runs away as depth approaches the far plane.
    let gap_world = max((blocker.x - coords.z) * cascade.depth_extent, 0.0);
    let penumbra_world = gap_world * inti.sun_softness;
    // The kernel is one texel wide at scale 1, so this is the penumbra
    // measured in kernel widths, floored at 1: below that the filter
    // stops hiding the shadow map's own aliasing and the edge steps.
    let scale = max(penumbra_world / max(cascade.texel_world_size, 1e-6), 1.0);

    return inti_sample_castano(coords.xy, coords.z, scale, bounds);
}

/// Occlusion at `world_position`, 1 = fully lit.
///
/// `to_light` is the unit vector toward the light, for the depth bias.
///
/// # Why two cascades near a boundary
///
/// Texel density and filter width both change at a split, so a hard
/// handover is a visible line across the ground wherever one runs. Bevy
/// samples the next cascade too across an overlap band and mixes — twice
/// the cost, in a band, and it is the difference between cascades you
/// cannot see and cascades you can point at.
fn inti_shadow(
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_light: vec3<f32>,
    view_depth: f32,
    n_dot_l: f32,
) -> f32 {
    if (inti.shadows_enabled == 0u) {
        return 1.0;
    }
    let picked = inti_pick_cascade(view_depth);
    let index = u32(picked.x);
    if (index >= 4u) {
        return 1.0;
    }

    var lit = inti_sample_cascade(index, world_position, normal, to_light, n_dot_l);
    if (picked.y <= 0.0) {
        return lit;
    }

    // Inside the overlap band. The last cascade has no successor, so it
    // fades to lit instead — a gradient into "no shadow data" rather
    // than an edge at the end of the world.
    if (index == 3u) {
        return mix(lit, 1.0, picked.y);
    }
    let next = inti_sample_cascade(index + 1u, world_position, normal, to_light, n_dot_l);
    return mix(lit, next, picked.y);
}

// Bevy's cascade colours, and their derivation: hue swept around the
// wheel by cascade index (`shadows.wgsl:265`). Ported rather than
// picked so a capture from this engine and one from Bevy read the same.
// `FRAC_PI_3` and `PI_2` are theirs too, from `bevy_render::maths`.
// Bevy divides the hue by `MAX_CASCADES_PER_LIGHT + 1` so the last
// cascade does not wrap onto the first one's colour.
const FRAME_CASCADE_COUNT_PLUS_ONE: u32 = 5u;
const INTI_FRAC_PI_3: f32 = 1.04719755;
const INTI_PI_2: f32 = 6.28318531;

// `bevy_render::color_operations::hsv_to_rgb`, transcribed.
// H ∈ [0, 2π), S ∈ [0, 1], V ∈ [0, 1].
fn inti_hsv_to_rgb(hsv: vec3<f32>) -> vec3<f32> {
    let n = vec3<f32>(5.0, 3.0, 1.0);
    let k = (n + hsv.x / INTI_FRAC_PI_3) % 6.0;
    return hsv.z - hsv.z * hsv.y * max(vec3<f32>(0.0), min(k, min(4.0 - k, vec3<f32>(1.0))));
}

/// What the shadow system sees at this point, as colour.
///
/// # Bevy's colour, and one thing on top
///
/// The hue is `cascade_debug_visualization`'s, computed the same way, so
/// "which cascade covers this" reads identically to a Bevy capture.
///
/// What Bevy does not answer, and #476 needed twice, is **whether the
/// map has an occluder over this point**: "the cascade does not reach
/// here", "the occluder was culled out of the map" and "the sampling is
/// wrong" are three different bugs that look like one missing shadow.
/// So the hue is dimmed where this point is shadowed.
///
/// 🔴 Dimmed by `inti_sample_cascade` — **the same call the shading pass
/// makes**, bias, filter and all. The previous version sampled the atlas
/// raw and deliberately without bias, to show the acne the shading
/// hides; what it actually showed was a screenful of moiré with the
/// cascade boundaries drowned underneath. A debug view whose own noise
/// hides its answer is not a debug view.
///
/// - magenta — no atlas: nothing casts
/// - black — inside no cascade volume, so nothing can be in shadow
/// - dark grey — past the last cascade
/// - cascade hue, bright — lit
/// - cascade hue, dim — shadowed, as the shading pass sees it
fn inti_shadow_debug(world_position: vec3<f32>, n: vec3<f32>, view_depth: f32) -> vec3<f32> {
    if (inti.shadows_enabled == 0u) {
        return vec3<f32>(1.0, 0.0, 1.0);
    }
    let picked = inti_pick_cascade(view_depth);
    let index = u32(picked.x);
    if (index >= 4u) {
        return vec3<f32>(0.15);
    }
    let cascade = inti.cascades[index];
    if (inti_shadow_coords(cascade, world_position).w == 0.0) {
        return vec3<f32>(0.0);
    }

    let hue = f32(index) / f32(FRAME_CASCADE_COUNT_PLUS_ONE) * INTI_PI_2;
    let colour = inti_hsv_to_rgb(vec3<f32>(hue, 1.0, 1.0));

    // The first directional light, which is the only one that casts
    // (#734 is the other half). Without one there is nothing to sample
    // against and the atlas check above already answered.
    for (var i = 0u; i < inti.light_count; i = i + 1u) {
        let light = inti_lights[i];
        if (light.kind != INTI_KIND_DIRECTIONAL) {
            continue;
        }
        let s = inti_sample_light(light, world_position);
        let n_dot_l = dot(n, s.to_light);
        if (n_dot_l <= 0.0) {
            // Facing away from the sun. Not shadowed — unlit, which is
            // a different answer and has a different fix.
            return colour * 0.12;
        }
        let lit = inti_sample_cascade(index, world_position, n, s.to_light, n_dot_l);
        return colour * mix(0.30, 1.0, lit);
    }
    return colour * 0.65;
}

/// The contact-shadow march, as colour, for the first light that opted
/// in (#735).
///
/// **One light, because the march is per light**: summing several would
/// average away the thing being looked at. The first opted-in light is
/// the sun in every scene that has one, which is the light whose
/// contact shadow anybody is inspecting.
///
/// The colours are `inti_contact_shadow_debug`'s and the reasoning is
/// there. Magenta here means *no light in the scene marches at all* —
/// which is a different answer from "it marched and found nothing", and
/// they look identical in a shaded frame.
fn inti_contact_shadow_debug_view(
    world_position: vec3<f32>,
    n: vec3<f32>,
    frag_coord: vec2<f32>,
) -> vec3<f32> {
    for (var i = 0u; i < inti.light_count; i = i + 1u) {
        let light = inti_lights[i];
        if ((light.flags & INTI_LIGHT_CONTACT_SHADOWS) == 0u) {
            continue;
        }
        let s = inti_sample_light(light, world_position);
        // Same gate the shading loop applies: a surface facing away
        // from the light is not marched, and painting it as "no hit"
        // would read as a failure of the march rather than as geometry.
        if (dot(n, s.to_light) <= 0.0) {
            return vec3<f32>(0.04);
        }
        let to_camera = normalize(inti.camera_position - world_position);
        return inti_contact_shadow_debug(
            inti_contact_shadow_probe(world_position, n, to_camera, s.to_light, frag_coord));
    }
    return vec3<f32>(1.0, 0.0, 1.0);
}

// The whole model, for one surface point.
//
// `base_color` is linear albedo (sRGB textures are decoded by the
// sampler; `Material::base_color` is documented linear). `metallic`
// and `roughness` are the usual perceptual [0,1] scalars.
//
// Returns linear HDR radiance. Exposure and the transfer function are
// applied by `inti_tonemap` separately, so a shadow pass or a debug
// overlay can consume the raw value.
fn inti_shade(
    world_position: vec3<f32>,
    n: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    frag_coord: vec2<f32>,
) -> vec3<f32> {
    // Distance along the view axis, which is what picks a cascade.
    //
    // Not the radial distance to the camera: that makes every cascade
    // boundary a sphere, so it crosses in the corners of the screen
    // before the centre and the split sweeps across the frame as the
    // camera turns. Projecting onto the forward axis makes the boundary
    // a plane, which is what the cascade fit assumes.
    let view_depth = dot(world_position - inti.camera_position, inti.camera_forward);
    let v = normalize(inti.camera_position - world_position);
    let n_dot_v = max(dot(n, v), 1e-4);

    // Metals have no diffuse and take F0 from their albedo;
    // dielectrics reflect a flat 4% and keep all of theirs.
    let diffuse_color = base_color * (1.0 - metallic);
    let f0 = mix(vec3<f32>(0.04), base_color, metallic);

    let perceptual = clamp(roughness, INTI_MIN_PERCEPTUAL_ROUGHNESS, 1.0);
    let a = perceptual * perceptual;
    let f_ab = inti_f_ab(perceptual, n_dot_v);

    var radiance = vec3<f32>(0.0);
    for (var i = 0u; i < inti.light_count; i = i + 1u) {
        let light = inti_lights[i];
        let s = inti_sample_light(light, world_position);
        let n_dot_l = dot(n, s.to_light);
        if (n_dot_l <= 0.0) {
            continue;
        }

        let h = normalize(s.to_light + v);
        let n_dot_h = saturate(dot(n, h));
        let l_dot_h = saturate(dot(s.to_light, h));

        let d = inti_d_ggx(a, n_dot_h);
        let vis = inti_v_smith_correlated(a, n_dot_v, n_dot_l);
        let f = inti_fresnel(f0, l_dot_h);
        let specular = inti_specular_multiscatter(d * vis * f, f0, f_ab);

        // Energy the specular layer reflected is energy the diffuse
        // layer underneath never receives. Bevy's forward path skips
        // this weighting; their path tracer does not, and it is the
        // path tracer that is right.
        let diffuse = (vec3<f32>(1.0) - f)
            * diffuse_color
            * inti_fd_burley(perceptual, n_dot_v, n_dot_l, l_dot_h);

        // Only the directional light casts today: the cascades are fit
        // to the view frustum for a light with no position, and a
        // punctual light needs a cube map or a projected map instead.
        // #476 is sun shadows; #734's light textures are the other half.
        var shadow = 1.0;
        if (light.kind == INTI_KIND_DIRECTIONAL) {
            shadow = inti_shadow(world_position, n, s.to_light, view_depth, n_dot_l);
        }

        // Contact shadows (#735) — the last few centimetres the cascades
        // cannot resolve, for any light kind, because a screen-space
        // march needs no shadow map and so has no reason to be the sun's
        // privilege. Skipped where the cascade already shadows this
        // point: multiplying two occlusions of the same occluder darkens
        // twice, and a march that finds nothing cannot brighten it back.
        if ((light.flags & INTI_LIGHT_CONTACT_SHADOWS) != 0u && shadow > 0.0) {
            shadow *= inti_contact_shadow(world_position, n, v, s.to_light, frag_coord);
        }

        radiance += (diffuse + specular) * s.irradiance * n_dot_l * shadow;
    }

    radiance += inti_ambient(n, diffuse_color, f0, f_ab);
    return radiance;
}

// ACES filmic approximation (Narkowicz 2015). Provisional: #254 owns
// the real tonemapper and the auto exposure that lets a sunlit surface
// and a planet's night side coexist in one frame.
fn inti_aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return saturate((x * (a * x + b)) / (x * (c * x + d) + e));
}

// Linear → sRGB electrical values.
//
// 🔴 NOT redundant with a hardware sRGB target. `GpuContext` picks a
// deliberately NON-sRGB surface format ("most renderers handle gamma
// correction in the shader") and until now no shader did. Skip this
// and a correctly-lit scene renders visibly too dark with crushed
// midtones — which reads as "the lighting is wrong" and is not.
fn inti_linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let cutoff = c < vec3<f32>(0.0031308);
    let low = c * 12.92;
    let high = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, cutoff);
}

// HDR radiance → the 8-bit value the surface expects.
fn inti_tonemap(radiance: vec3<f32>) -> vec3<f32> {
    return inti_linear_to_srgb(inti_aces(radiance * inti.exposure));
}

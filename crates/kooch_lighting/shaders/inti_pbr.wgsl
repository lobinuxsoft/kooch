// The shading model, ported from Bevy 0.19's `pbr_lighting.wgsl` and shared by every path via
// `{{INTI_GROUP}}`. Carries bevy#22454, bevy#22372 and Solari's `(1 - F)` diffuse weight.
// `D_GGX`/`V_Smith` take linear roughness, `F_AB` perceptual.

const INTI_PI: f32 = 3.14159265359;

const INTI_KIND_DIRECTIONAL: u32 = 0u;
const INTI_KIND_POINT: u32 = 1u;
const INTI_KIND_SPOT: u32 = 2u;

// Filament's f32 floor on perceptual roughness. Below it the GGX lobe is a delta function: a
// punctual light either misses it entirely or lands one blinding pixel on it.
const INTI_MIN_PERCEPTUAL_ROUGHNESS: f32 = 0.089;

// Mirror of `GpuLight` (80 B); field order and padding are load-bearing, and `gpu_light.rs` pins
// the size.
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
    // Cone falloff as a multiply-add, as Bevy packs it: `cos_outer = -offset / scale`, `cos_inner =
    // cos_outer + 1 / scale`.
    spot_scale: f32,
    spot_offset: f32,
    // Per-light opt-in bits, in a pad word, so fifty lights do not pay fifty screen-space marches
    // per pixel.
    flags: u32,
    // Index into `inti.spot_shadows` for a spot light that casts, or INTI_NO_SHADOW_SLOT (#777).
    shadow_slot: u32,
    // Radius of the emitting sphere in world units, 0 for a point.
    // Specular only — see `inti_representative_point`.
    radius: f32,
    // 🔴 THREE SCALARS, never a vec3: a vec3 aligns to 16 and would push the struct to 96 while
    // Rust still writes 80. Same trap the cascade descriptor documents above.
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

// `IntiLight.shadow_slot` when the light casts no shadow.
const INTI_NO_SHADOW_SLOT: u32 = 0xffffffffu;

// Bit 0 of `IntiSurface.flags` — this surface samples shadow maps (#804). Mirrors
// `INSTANCE_RECEIVES_SHADOWS` on the Rust side; the two are one bit in one place and have to agree.
const INTI_SURFACE_RECEIVES_SHADOWS: u32 = 1u;

// Bit 0 of `IntiLight.flags` — this light marches for contact shadows.
// Mirrors `GpuLight::FLAG_CONTACT_SHADOWS`.
const INTI_LIGHT_CONTACT_SHADOWS: u32 = 1u;

// One cascade: where it lives in the atlas and how to get there.
struct IntiCascade {
    // Light-space clip-from-world.
    view_proj: mat4x4<f32>,
    // Which layer of the shadow array this cascade rendered into. Was a quadrant transform into a
    // single atlas texture; an array layer is one binding all the same and leaves the uv untouched.
    layer: u32,
    // 🔴 Three scalars, never `vec3<u32>`: it aligns to 16 and grows each cascade by 16 B, surfacing
    // only as `min_binding_size` rejecting the pipeline.
    _pad_layer0: u32,
    _pad_layer1: u32,
    _pad_layer2: u32,
    // View-space depth past which the next cascade takes over.
    far_depth: f32,
    // World units per shadow texel, for the filter radius and the penumbra estimate. A fixed radius
    // in texels is a different distance in every cascade.
    texel_world_size: f32,
    // World units spanned by [0, 1] depth, turning a depth difference into the metres a penumbra is
    // proportional to.
    depth_extent: f32,
    _pad0: f32,
}

// Mirror of `kooch_lighting::GpuPointShadow` (#778). Sixteen bytes and no matrix: a cube map is
// sampled by DIRECTION, so the only transform is the subtraction the shader already does.
struct IntiPointShadow {
    // Near plane of the six faces; with infinite reverse-Z the stored depth is `near / major_axis`,
    // one scalar for Bevy's four terms.
    near: f32,
    // Texel size per METRE of distance, like the spots'. A cube face is
    // 90°, so it is `2 / size` and never involves the light's range.
    texel_world_size: f32,
    depth_extent: f32,
    _pad0: f32,
}

/// Per-frame lighting constants; `camera_position` is here because the shared camera UBO is pinned
/// at 64 B.
struct IntiFrame {
    ambient_sky: vec3<f32>,
    light_count: u32,
    ambient_ground: vec3<f32>,
    // 1 / (2^EV100 * 1.2). Fixed until auto exposure lands (#254). Without it a 10 000 lux sun
    // clips every channel to white and the whole model reads as "broken" rather than "unexposed".
    exposure: f32,
    camera_position: vec3<f32>,
    ambient_intensity: f32,
    // Unit vector down the view axis. Only the shadow cascades need it, and they need it to be the
    // axis rather than the radial direction — see `inti_shade`.
    camera_forward: vec3<f32>,
    _pad_forward: f32,
    // The four cascades. Fixed-size because the count is baked into the
    // atlas layout: changing it is a texture change, not a loop bound.
    cascades: array<IntiCascade, 4>,
    // One per casting spot (#777), reusing the cascade record: `inti_shadow_coords` already divides
    // by w.
    spot_shadows: array<IntiCascade, 4>,
    spot_shadow_count: u32,
    // #778's count, in a word the spot count's padding already had.
    point_shadow_count: u32,
    // Irradiance below which a light skips its specular layer (#821).
    // Zero keeps every light on the full model.
    specular_floor: f32,
    // How many of a froxel's punctual lights a pixel may evaluate, or 0
    // for all of them. A measuring instrument: it drops real light, and
    // it exists to answer whether the cost scales with that count.
    light_limit: u32,
    point_shadows: array<IntiPointShadow, {{INTI_MAX_POINT_SHADOWS}}>,
    // 0 when no directional light casts, or the atlas has not been rendered. The dummy 1x1 atlas
    // bound in that case would return "fully lit" anyway; the flag skips the work.
    shadows_enabled: u32,
    // Fraction of a split distance over which one cascade fades into the next. Without it the
    // boundary is a visible line where texel density and filter radius change at once.
    cascade_blend: f32,
    // Tangent of the sun's angular RADIUS: how much wider a shadow gets
    // per metre of gap between blocker and receiver. An angle, not a
    // width, because that is what a light infinitely far away has.
    sun_softness: f32,
    // The single-light view's light (#743), `>= light_count` for none — in former tail padding,
    // since the group is full.
    debug_light: u32,
    // The view matrix's third row: one dot product turns a world
    // position into the view depth that picks a froxel slice (#780).
    view_z_row: vec4<f32>,
    // xyz = grid dimensions, w = their product.
    cluster_dimensions: vec4<u32>,
    // xy = grid cells per pixel, zw = the logarithmic slice constants.
    cluster_factors: vec4<f32>,
    // How long the index list is, for the loop to clamp against.
    cluster_capacity: u32,
    // Directional lights, which the grid does not cluster — they reach every cell. The first
    // entries of the light buffer, walked linearly.
    directional_count: u32,
    // 0 when no grid was built this frame: shading falls back to the linear walk over every light.
    clustered: u32,
    // Top of scale for the lights-per-pixel debug view (#817). Read only
    // by `inti_debug.wgsl`; the production pipeline never touches it.
    debug_lights_hot: u32,
    _pad_samples0: u32,
    _pad_samples1: u32,
    _pad_samples2: u32,
    _pad_samples3: u32,
}

// One froxel's record. Mirrors `ClusterCell` in `cluster_common.wgsl`,
// minus the atomics — nothing here writes.
struct IntiClusterCell {
    offset: u32,
    point_count: u32,
    spot_count: u32,
    probe_count: u32,
    volume_count: u32,
    decal_count: u32,
    _pad0: u32,
    _pad1: u32,
}

@group({{INTI_GROUP}}) @binding(0) var<uniform> inti: IntiFrame;
// Always at least one element: wgpu rejects a zero-sized storage binding, so an unlit scene binds a
// one-element buffer with `light_count == 0` rather than needing a second pipeline.
@group({{INTI_GROUP}}) @binding(1) var<storage, read> inti_lights: array<IntiLight>;

// Cascaded shadow maps (#476) in Inti's group — the bind-group budget is spent. A comparison
// sampler: the hardware returns filtered occlusion, bilinear PCF for free.
@group({{INTI_GROUP}}) @binding(2) var inti_shadow_atlas: texture_depth_2d_array;
@group({{INTI_GROUP}}) @binding(3) var inti_shadow_sampler: sampler_comparison;
// A non-comparison sampler on the same texture for the blocker search, which needs the depth itself
// — Bevy's same pair.
@group({{INTI_GROUP}}) @binding(4) var inti_shadow_point_sampler: sampler;
// The point lights' cube array (#778), six layers per light. 🔴 A binding, not a group; the samplers
// above are reused, since a sampler is not tied to a texture.
@group({{INTI_GROUP}}) @binding(5) var inti_point_cubes: texture_depth_cube_array;

// The froxel grid (#780), two more bindings here for the same reason as the shadow maps.
@group({{INTI_GROUP}}) @binding(6) var<storage, read> inti_clusters: array<IntiClusterCell>;
@group({{INTI_GROUP}}) @binding(7) var<storage, read> inti_cluster_indices: array<u32>;

// Virtual shadow maps (#866); the page helpers come from `page_table.wgsl`, since the passes
// filling this table live in another crate and must decode ids identically.
@group({{INTI_GROUP}}) @binding(8) var<uniform> inti_pages: PageRaster;
// The FLAT page table: `PAGE_CELL` words per virtual page, indexed by
// the page id itself — `slot + 1` first, `PAGE_ABSENT` meaning "not
// resident". Binding 9 held the hash's key array and is retired.
@group({{INTI_GROUP}}) @binding(10) var<storage, read> inti_page_slots: array<u32>;
// 🔴 `textureLoad`, never a sampler: a filter cannot stop at a page border. An array with a layer
// per view, so two viewports share the pool without clearing each other's pages.
@group({{INTI_GROUP}}) @binding(11) var inti_page_atlas: texture_depth_2d_array;

// GGX / Trowbridge-Reitz, in Filament's reassociated form. The naïve
// `a2 / (π·((NdotH²)(a2-1)+1)²)` loses catastrophic precision in f32 at
// low roughness — the highlight breaks into blocks. `a` is linear.
fn inti_d_ggx(a: f32, n_dot_h: f32) -> f32 {
    let one_minus_n_dot_h_sq = 1.0 - n_dot_h * n_dot_h;
    let x = n_dot_h * a;
    let k = a / (one_minus_n_dot_h_sq + x * x);
    return k * k * (1.0 / INTI_PI);
}

// Height-correlated Smith (Heitz 2014), returning G / (4·NoV·NoL) — do not divide again, or grazing
// angles go black. `a` is linear.
fn inti_v_smith_correlated(a: f32, n_dot_v: f32, n_dot_l: f32) -> f32 {
    let a2 = a * a;
    let lambda_v = n_dot_l * sqrt((n_dot_v - a2 * n_dot_v) * n_dot_v + a2);
    let lambda_l = n_dot_v * sqrt((n_dot_l - a2 * n_dot_l) * n_dot_l + a2);
    return 0.5 / max(lambda_v + lambda_l, 1e-4);
}

fn inti_f_schlick_scalar(f0: f32, f90: f32, v_dot_h: f32) -> f32 {
    return f0 + (f90 - f0) * pow(saturate(1.0 - v_dot_h), 5.0);
}

// f90 derived from f0 rather than assumed to be 1.0. A near-black dielectric with f90 = 1 grows a
// white rim at grazing angles that no real material has; the 50·0.33 scale is Filament's fit.
fn inti_fresnel(f0: vec3<f32>, v_dot_h: f32) -> vec3<f32> {
    let f90 = saturate(dot(f0, vec3<f32>(50.0 * 0.33)));
    return f0 + (vec3<f32>(f90) - f0) * pow(saturate(1.0 - v_dot_h), 5.0);
}

// Analytic fit to the split-sum DFG integral (Karis / Lazarov), the same polynomial Bevy falls back
// to without a DFG LUT. Feeds the multiscatter compensation below. Takes PERCEPTUAL roughness.
fn inti_f_ab(perceptual_roughness: f32, n_dot_v: f32) -> vec2<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = perceptual_roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    return max(vec2<f32>(-1.04, 1.04) * a004 + r.zw, vec2<f32>(0.00005));
}

// Single-scattering GGX loses energy on rough metals — they go grey instead of staying bright.
// Compensates by the fraction the split-sum integral says went missing.
fn inti_specular_multiscatter(
    single_scatter: vec3<f32>,
    f0: vec3<f32>,
    f_ab: vec2<f32>,
) -> vec3<f32> {
    return single_scatter * (1.0 + f0 * (1.0 / (f_ab.x + f_ab.y) - 1.0));
}

// Burley / Disney diffuse. Lambert is flat; this one brightens the grazing edge on rough surfaces
// the way cloth and unfinished wood actually do. `a` unused — Burley takes perceptual roughness.
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

// Inverse-square with a smooth window reaching exactly zero at `range`, so the gizmo's wire sphere
// is the truth and nothing pops.
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
    // Surface to light, unnormalised. The representative point (#776) works in this space, and
    // recomputing it there would be the second subtraction of the same two vectors.
    offset: vec3<f32>,
    // Length of `offset`. Zero for a directional light, which has none.
    distance: f32,
}

fn inti_sample_light(light: IntiLight, world_position: vec3<f32>) -> IntiSample {
    var out: IntiSample;
    if (light.kind == INTI_KIND_DIRECTIONAL) {
        // Infinitely far away: no falloff, and illuminance in lux IS the perpendicular irradiance.
        out.to_light = -light.direction;
        out.irradiance = light.color * light.intensity;
        out.offset = out.to_light;
        out.distance = 0.0;
        return out;
    }

    let offset = light.position - world_position;
    let distance_sq = dot(offset, offset);
    out.to_light = offset * inverseSqrt(max(distance_sq, 1e-8));
    out.offset = offset;
    out.distance = sqrt(distance_sq);

    // Lumens → candela over the full sphere, spots included: narrowing a cone aims a light, it must
    // not brighten it (Unity and Bevy agree).
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

// Hemisphere ambient, standing in for IBL (#450) so metals are not black. ⚠️ `n.y` is world up,
// meaningless across a planet; the fix is a probe.
fn inti_ambient(
    n: vec3<f32>,
    diffuse_color: vec3<f32>,
    f0: vec3<f32>,
    f_ab: vec2<f32>,
) -> vec3<f32> {
    let t = n.y * 0.5 + 0.5;
    let sky = mix(inti.ambient_ground, inti.ambient_sky, t) * inti.ambient_intensity;
    // Split-sum specular against a uniform environment: `F0·f_ab.x + f_ab.y` is the exact DFG
    // answer here, not a fudge.
    let specular = f0 * f_ab.x + f_ab.y;
    // Diffuse gets what specular did not reflect, weighted by the same term; a Schlick at N·V
    // instead stops the halves summing to one.
    return sky * (diffuse_color * (vec3<f32>(1.0) - specular) + specular);
}

// Shadows (#476) — PCSS: blocker search, penumbra width from the receiver–blocker gap, then PCF at
// that width. The varying width is what reads as a shadow instead of a blurred stencil.




// Which cascade covers `view_depth`, and how far into the blend band it is. `x` is the index, `y`
// is 0 inside the cascade and rises to 1 at its far edge.
fn inti_pick_cascade(view_depth: f32) -> vec2<f32> {
    for (var i = 0u; i < 4u; i = i + 1u) {
        let far = inti.cascades[i].far_depth;
        if (view_depth < far) {
            let band = far * inti.cascade_blend;
            let blend = select(0.0, (view_depth - (far - band)) / max(band, 1e-4), band > 0.0);
            return vec2<f32>(f32(i), saturate(blend));
        }
    }
    // Past the last cascade there is nothing to sample. Reported as index 4 so the caller returns
    // fully lit rather than clamping to the last cascade and stretching its shadow to the horizon.
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
    // NDC to uv: x maps directly, y flips because texture space counts down from the top.
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
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

/// World-space depth bias toward the light, Bevy's `DEFAULT_SHADOW_DEPTH_BIAS`; no rasterizer bias
/// besides, since two biases detach shadows and still leave acne. Spots share it.
const INTI_DEPTH_BIAS: f32 = 0.02;

/// Normal offset in shadow texels, Bevy's `DEFAULT_SHADOW_NORMAL_BIAS`: moving along the surface
/// stops acne on grazing faces without detaching the shadow.
const INTI_NORMAL_BIAS: f32 = 1.8;

/// 🔴 Point lights use Bevy's own 0.08 / 0.6: with the sun's pair cubes had a quarter of the depth
/// bias and printed a self-shadow square under every lamp. Coarse 90° texels want more depth push,
/// less normal push.
const INTI_POINT_DEPTH_BIAS: f32 = 0.08;
const INTI_POINT_NORMAL_BIAS: f32 = 1.8;

// World units → shadow uv for a cascade: `1 / (texel_size × layer_size)`, as Bevy. A filter radius
// is honest in metres, not texels.
fn inti_world_to_atlas_uv(cascade: IntiCascade) -> f32 {
    let layer_size = f32(textureDimensions(inti_shadow_atlas).x);
    return 1.0 / max(cascade.texel_world_size * layer_size, 1e-6);
}

// Average stored depth of whatever is between this point and the light,
// and whether there was anything. Under reversed-Z an occluder is
// CLOSER to the light and therefore stored GREATER than the receiver.
fn inti_blocker_depth(
    uv: vec2<f32>,
    layer: u32,
    receiver_depth: f32,
    radius_uv: f32,
    bounds: vec4<f32>,
) -> vec2<f32> {
    var sum = vec2<f32>(0.0);
    for (var i = 0; i < 8; i = i + 1) {
        let tap = clamp(uv + INTI_BLOCKER_TAPS[i] * radius_uv, bounds.xy, bounds.zw);
        // A plain sample. The level is an integer on a depth texture; `0.0` fails naga with
        // `InvalidSampleLevelExactType`.
        let depth = textureSampleLevel(
            inti_shadow_atlas, inti_shadow_point_sampler, tap, layer, 0u);
        sum += select(vec2<f32>(0.0), vec2<f32>(depth, 1.0), depth > receiver_depth);
    }
    if (sum.y == 0.0) {
        return vec2<f32>(0.0, 0.0);
    }
    return vec2<f32>(sum.x / sum.y, 1.0);
}

// Castano '13 (Bevy's default): nine bilinear taps weighted into a grid-aligned 5×5 Gaussian,
// replacing 16 ringing Poisson taps (no TAA, #732). `scale` widens it for PCSS; `bounds` clamps
// taps off other cascades.
fn inti_sample_castano(
    uv: vec2<f32>,
    layer: u32,
    depth: f32,
    scale: f32,
    bounds: vec4<f32>,
) -> f32 {
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
                inti_shadow_atlas, inti_shadow_sampler, tap, layer, depth);
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
    return inti_sample_cascade_record(
        inti.cascades[index], world_position, normal, to_light, n_dot_l, 1.0);
}

/// Sampling over a record rather than a cascade index, so spots (#777) share the bias, blocker
/// search, Castano filter and clamp.
fn inti_sample_cascade_record(
    cascade: IntiCascade,
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_light: vec3<f32>,
    n_dot_l: f32,
    // Scales `texel_world_size`: 1 for an orthographic cascade, the fragment's distance for a spot,
    // whose texel is an angle.
    texel_scale: f32,
) -> f32 {
    let texel_world = cascade.texel_world_size * texel_scale;

    // Bevy's two world-space offsets, no rasterizer bias; the normal offset scales with the texel
    // only — a slope factor doubled it where detachment shows most.
    let offset_position = world_position
        + normal * (texel_world * INTI_NORMAL_BIAS)
        + to_light * INTI_DEPTH_BIAS;

    let coords = inti_shadow_coords(cascade, offset_position);
    if (coords.w == 0.0) {
        return 1.0;
    }

    let to_uv = 1.0 / max(texel_world * f32(textureDimensions(inti_shadow_atlas).x), 1e-6);
    // The layer inset by half a texel, so a bilinear tap stays inside [0, 1] by intent rather than
    // sampler rule.
    let half_texel = 0.5 / f32(textureDimensions(inti_shadow_atlas).x);
    let bounds = vec4<f32>(
        vec2<f32>(half_texel),
        vec2<f32>(1.0 - half_texel),
    );

    // 1. Blocker search, over a disc as wide as the softest penumbra the
    // sun could produce across this cascade's depth range.
    let search_world = max(
        inti.sun_softness * cascade.depth_extent, texel_world * 2.0);
    let blocker = inti_blocker_depth(
        coords.xy, cascade.layer, coords.z, search_world * to_uv, bounds);
    if (blocker.y == 0.0) {
        return 1.0;
    }

    // 2. Penumbra: reversed-Z, so the blocker is greater, scaled to metres by the depth extent. ⚠️
    // Not Bevy's perspective divide — for an orthographic cascade it tracks distance from the sun.
    let gap_world = max((blocker.x - coords.z) * cascade.depth_extent, 0.0);
    let penumbra_world = gap_world * inti.sun_softness;
    // The kernel is one texel wide at scale 1, so this is the penumbra
    // measured in kernel widths, floored at 1: below that the filter
    // stops hiding the shadow map's own aliasing and the edge steps.
    let scale = max(penumbra_world / max(texel_world, 1e-6), 1.0);

    return inti_sample_castano(coords.xy, cascade.layer, coords.z, scale, bounds);
}


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
//   bright with distance. This note used to end "our punctual lights
//   have no radius, so there is no `a_prime` to get wrong yet". #776
//   gave them one: `inti_representative_point` produces the `a_prime`,
//   and everything that reads it in `inti_light_contribution` is the
//   fix, not an embellishment of it.
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

// Mirror of `kooch_lighting::GpuLight` (80 B, `#[repr(C)]`). Field
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
    // Index into `inti.spot_shadows` for a spot light that casts, or
    // INTI_NO_SHADOW_SLOT (#777).
    shadow_slot: u32,
    // Radius of the emitting sphere in world units, 0 for a point.
    // Specular only — see `inti_representative_point`.
    radius: f32,
    // 🔴 THREE SCALARS, never a vec3: a vec3 aligns to 16 and would
    // push the struct to 96 while Rust still writes 80. Same trap the
    // cascade descriptor documents above.
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

// `IntiLight.shadow_slot` when the light casts no shadow.
const INTI_NO_SHADOW_SLOT: u32 = 0xffffffffu;

// Bit 0 of `IntiSurface.flags` — this surface samples shadow maps
// (#804). Mirrors `INSTANCE_RECEIVES_SHADOWS` on the Rust side; the two
// are one bit in one place and have to agree.
const INTI_SURFACE_RECEIVES_SHADOWS: u32 = 1u;

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
    // Which layer of the shadow array this cascade rendered into.
    // Was a quadrant transform into a single atlas texture; an array
    // layer is one binding all the same and leaves the uv untouched.
    layer: u32,
    // 🔴 THREE SCALARS, never `vec3<u32>`. A vec3 aligns to 16, which
    // pushes this field to the next boundary and grows the struct by 16
    // bytes per cascade — 464 → 528 for the frame, and the only symptom
    // is `min_binding_size` rejecting the pipeline. Same trap the GDF
    // cascade descriptor hit.
    _pad_layer0: u32,
    _pad_layer1: u32,
    _pad_layer2: u32,
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

// Mirror of `kooch_lighting::GpuPointShadow` (#778). Sixteen bytes and
// no matrix: a cube map is sampled by DIRECTION, so the only transform
// is the subtraction the shader already does.
struct IntiPointShadow {
    // The near plane the six faces rendered with. With an infinite
    // reverse-Z projection the stored depth is exactly
    // `near / major_axis_magnitude`, which is why Bevy's four projection
    // terms collapse to this one scalar.
    near: f32,
    // Texel size per METRE of distance, like the spots'. A cube face is
    // 90°, so it is `2 / size` and never involves the light's range.
    texel_world_size: f32,
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
    // One per shadow-casting spot light (#777), in the same record the
    // cascades use: `inti_shadow_coords` already divides by w, which is
    // exactly what a spot's perspective needs and a cascade's
    // orthographic does not.
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
    // Which light the single-light debug view isolates (#743). Anything
    // `>= light_count` means none. It rides in what was this struct's
    // tail padding: the view costs no binding and no byte, which is the
    // only way it was going to fit — Inti's group is full and there is
    // no seventh.
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
    // Directional lights, which the grid does not cluster — they reach
    // every cell. The first entries of the light buffer, walked
    // linearly.
    directional_count: u32,
    // 0 when no grid was built this frame: shading falls back to the
    // linear walk over every light.
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
@group({{INTI_GROUP}}) @binding(2) var inti_shadow_atlas: texture_depth_2d_array;
@group({{INTI_GROUP}}) @binding(3) var inti_shadow_sampler: sampler_comparison;
// A second sampler on the SAME texture, non-comparison, for the blocker
// search: it needs the stored depth, and a comparison sampler only ever
// answers "nearer or not". Bevy binds exactly this pair
// (`directional_shadow_textures_linear_sampler`).
@group({{INTI_GROUP}}) @binding(4) var inti_shadow_point_sampler: sampler;
// The point lights' cube array (#778): six layers per light, the light
// chosen by index and the face by the direction.
//
// 🔴 A fifth BINDING in this group, not a seventh group. The six-group
// budget is spent; bindings are not groups, and the two samplers above
// are reused unchanged because a sampler is not bound to a texture.
@group({{INTI_GROUP}}) @binding(5) var inti_point_cubes: texture_depth_cube_array;

// The froxel grid (#780): which lights reach each cell, and where each
// cell's run of indices starts. A sixth and seventh BINDING in this
// group, for the same reason the shadow maps are here — the six-group
// budget is spent and a light list without its lights is nothing any
// shader wants.
@group({{INTI_GROUP}}) @binding(6) var<storage, read> inti_clusters: array<IntiClusterCell>;
@group({{INTI_GROUP}}) @binding(7) var<storage, read> inti_cluster_indices: array<u32>;

// Virtual shadow maps (#866). `PageRaster`, `page_decode`, `sun_basis`,
// `sun_page_rect` and `page_origin` all come from `page_table.wgsl`,
// concatenated ahead of this file — the same
// arrangement the froxel grid uses, and for the same reason: the four
// passes that FILL this table live in another crate, and a page id
// encoded one way and decoded another samples somebody else's shadow.
@group({{INTI_GROUP}}) @binding(8) var<uniform> inti_pages: PageRaster;
// The FLAT page table: `PAGE_CELL` words per virtual page, indexed by
// the page id itself — `slot + 1` first, `PAGE_ABSENT` meaning "not
// resident". Binding 9 held the hash's key array and is retired.
@group({{INTI_GROUP}}) @binding(10) var<storage, read> inti_page_slots: array<u32>;
// 🔴 `textureLoad`, never a sampler. A filter cannot cross a page
// border: the neighbouring texels belong to another clipmap level, and
// hardware filtering has no way to be told where a page ends.
//
// 🔴 An ARRAY, with a layer per view. A layer is an attachment a camera
// can clear on its own, which is what lets two viewports share one pool
// without one of them wiping the pages the other is still sampling.
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
    // Surface to light, unnormalised. The representative point (#776)
    // works in this space, and recomputing it there would be the second
    // subtraction of the same two vectors.
    offset: vec3<f32>,
    // Length of `offset`. Zero for a directional light, which has none.
    distance: f32,
}

fn inti_sample_light(light: IntiLight, world_position: vec3<f32>) -> IntiSample {
    var out: IntiSample;
    if (light.kind == INTI_KIND_DIRECTIONAL) {
        // Infinitely far away: no falloff, and illuminance in lux IS
        // the perpendicular irradiance.
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

/// Depth pushed along the direction to the light, in world units.
///
/// Bevy's `DirectionalLight::DEFAULT_SHADOW_DEPTH_BIAS`, and it does
/// this in world space rather than through the rasteriser's depth bias —
/// which is why the shadow pipeline here no longer sets one. Two biases
/// doing the same job is how a shadow ends up detached from its object
/// while still showing acne somewhere else.
///
/// Shared with the spots, which is Bevy's arrangement too: their
/// `SpotLight` defaults are this same pair.
const INTI_DEPTH_BIAS: f32 = 0.02;

/// How far along its own normal a surface's sample point is pushed, in
/// shadow texels. Bevy's `DirectionalLight::DEFAULT_SHADOW_NORMAL_BIAS`.
///
/// A surface edge-on to the light covers many world units inside one
/// shadow texel, and no constant depth bias covers that. Moving along
/// the surface instead of pushing depth is what stops acne without
/// detaching the shadow.
const INTI_NORMAL_BIAS: f32 = 1.8;

/// 🔴 A point light gets its **own** pair, and this file used to hand it
/// the two above.
///
/// Bevy has three light types and only two sets of numbers: the
/// directional and the spot share `0.02 / 1.8`, and the point is the one
/// that differs — `PointLight::DEFAULT_SHADOW_DEPTH_BIAS = 0.08` with
/// `DEFAULT_SHADOW_NORMAL_BIAS = 0.6`. Borrowing the sun's pair ran the
/// cubes at a **quarter** of the depth bias they need.
///
/// # What that looked like
///
/// An empty floor under one lamp — nothing in the scene that could cast
/// anything — came out with a hard stair-stepped square printed on it,
/// which is the floor shadowing itself. The square is where the bias
/// changes regime: a cube face measures depth along the largest axis, so
/// below the lamp that distance is the lamp's height and constant, and
/// past it the distance starts growing with the ground offset. The two
/// sides of that boundary get different bias and the seam between them
/// is a square, centred under the lamp.
///
/// Why the numbers go opposite ways is the geometry: a cube face is 90°
/// and its texels are coarse, so the point needs more depth push, while
/// a lower normal push keeps a shadow from crawling away from its
/// object across six faces that meet at 45°.
const INTI_POINT_DEPTH_BIAS: f32 = 0.08;
const INTI_POINT_NORMAL_BIAS: f32 = 1.8;

// World units → shadow uv, for this cascade.
//
// One layer spans `texel_world_size × layer_size` world units across its
// whole `[0,1]`, so the ratio is that reciprocal. Same relation Bevy
// uses (`1 / (texel_size × shadow_map_size)`) — a distance in metres is
// the one honest unit for a filter radius, and texels are not.
//
// `textureDimensions` on an array texture reports one layer, which is
// what this wants; under the old atlas it reported the whole 2×2 sheet
// and the quadrant scale had to cancel it back out.
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
        // A plain sample, not a comparison one — see the binding. The
        // level is an INTEGER on a depth texture; 0.0 is
        // `InvalidSampleLevelExactType` out of naga, which reads like a
        // sampler problem and is a literal one.
        let depth = textureSampleLevel(
            inti_shadow_atlas, inti_shadow_point_sampler, tap, layer, 0u);
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

/// The sampling itself, over a record rather than an index into the
/// cascades.
///
/// Split out for spot lights (#777): a spot's shadow is the same record
/// in a different array, so this way the bias, the blocker search, the
/// Castano filter and the border clamp are one implementation instead of
/// two that drift.
fn inti_sample_cascade_record(
    cascade: IntiCascade,
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_light: vec3<f32>,
    n_dot_l: f32,
    // Multiplies `texel_world_size`. 1 for a cascade, whose orthographic
    // texel is the same size everywhere; a spot's own distance to this
    // fragment for a spot, whose texel is an ANGLE and only becomes a
    // length once something says how far away it is.
    texel_scale: f32,
) -> f32 {
    let texel_world = cascade.texel_world_size * texel_scale;

    // Both of Bevy's offsets, in world space, and no rasteriser depth
    // bias to go with them. The normal term scales with how obliquely
    // the light hits, because that is when one texel covers the most
    // surface.
    // No slope term. Bevy scales the normal offset by the cascade's
    // texel size and nothing else, and the extra factor was mine: it
    // doubles the offset on grazing surfaces, which is exactly where a
    // shadow detaching from its object is most visible.
    let offset_position = world_position
        + normal * (texel_world * INTI_NORMAL_BIAS)
        + to_light * INTI_DEPTH_BIAS;

    let coords = inti_shadow_coords(cascade, offset_position);
    if (coords.w == 0.0) {
        return 1.0;
    }

    let to_uv = 1.0 / max(texel_world * f32(textureDimensions(inti_shadow_atlas).x), 1e-6);
    // The layer, inset by half a texel so a bilinear tap on the border
    // cannot reach past it. Under the old atlas this clamped to the
    // cascade's quadrant, because a tap over the edge landed in a
    // DIFFERENT cascade's depths and shadowed with them. A layer has no
    // neighbour to bleed from — the clamp stays because a tap outside
    // [0,1] still wraps or clamps by sampler rule rather than by intent.
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
    let scale = max(penumbra_world / max(texel_world, 1e-6), 1.0);

    return inti_sample_castano(coords.xy, cascade.layer, coords.z, scale, bounds);
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
// Where a virtual page lives, or `PAGE_MISS`.
//
// ONE indexed read — the whole point of #477's flat table. This runs
// per pixel PER LIGHT, and the open-addressed walk it replaced (up to
// 32 probes, times up to 5 chain levels) measured 10.4 ms of shading
// against 0.884 ms for the entire shadow track. Chalmers, Stephano and
// UE5 all land on the same shape: a single lookup in the final pass.
fn inti_page_lookup(page: u32) -> u32 {
    if page >= inti_pages.pool.x {
        return PAGE_MISS;
    }
    let stored = inti_page_slots[page * PAGE_CELL];
    if stored == PAGE_ABSENT {
        return PAGE_MISS;
    }
    // 🔴 Resident is not the same as READABLE, and treating them as the
    // same is a lit hole.
    //
    // The fourth word is the content stamp and `PAGE_CELL` says what
    // zero means: no valid content. A page reaches that state by being
    // freshly claimed, by being invalidated, or by its bucket
    // overflowing so the compaction never listed it — and in all three
    // the atlas under its slot holds whatever was there before, or a
    // clear. A clear is far depth under reversed-Z, so every reader
    // over it answers "nothing occludes here".
    //
    // Reporting it as a hit also stops the caller's walk. `inti_page_
    // shadow` climbs the clipmap until a level answers, which is
    // Unreal's "onwards to coarser levels if no valid data is present"
    // — but only if this says no. A page that is present and empty
    // ended the search at the one level that could not answer, with a
    // coarser one right above it holding the shadow.
    //
    // Non-zero and not the generation is still readable: the content is
    // from an older generation the compaction chose not to redraw.
    // `PAGE_EMPTY` is non-zero on purpose and means "cleared", which is
    // a true answer — a lamp page with no caster in reach occludes
    // nothing — so it reads as a hit, and must.
    if inti_page_slots[page * PAGE_CELL + 3u] == 0u {
        return PAGE_MISS;
    }
    return stored - 1u;
}

/// Occlusion at `texel` inside one page, bilinear-PCF filtered.
///
/// # 🔴 Weighted, not averaged — this is why the shadow stopped looking
/// like the geometry
///
/// The first reader averaged four binary taps flat, and the measured
/// look was "the shadow IS the mesh": a hard staircase at texel scale,
/// against the smooth edge every other shadow path resolves. The cube
/// path got its smoothness for free — `textureSampleCompareLevel` is
/// comparison-BILINEAR in hardware. A page cannot use that sampler,
/// because hardware filtering cannot be told where a page ends: the
/// neighbour texel may belong to another level or another light. So
/// the same filter is done by hand — the taps CLAMP to the page and
/// only the weights cross the border. Costs nothing: the four loads
/// were already paid for.
///
/// # The footprint is configurable (#941)
///
/// `world.w` carries the box width `W` in texels, from
/// `RenderSettings::shadow_softness`. `W = 1` is the bilinear above,
/// bit for bit. Wider widths are the Castano-class box filter: the 1D
/// weights are `frac`-clipped at both ends and `1` in the middle, so
/// their sum is exactly `W` per axis and the kernel is a `W`-texel box
/// positioned with sub-texel precision — a penumbra that moves
/// smoothly, `(W + 1)²` loads. No blocker search: the width is the
/// author's, uniform across the scene, which is the price of paying
/// per light per pixel.
fn inti_page_filter(
    origin: vec2<f32>,
    layer: i32,
    texel: vec2<f32>,
    receiver: f32,
    // The receiver's depth gradient per texel (#1017). Zero puts every
    // tap back on the pixel's own depth, which is what a scalar bias
    // assumes and what the lamps still pass.
    slope: vec2<f32>,
    page_texels: u32,
    // The page's own identity on the sun's clipmap, so a tap that walks
    // off its edge can find the page it landed in. `PAGE_UNLISTED` as
    // the level means "clamp instead" — the lamps, whose grid is six
    // faces of a chain and not one wrapped plane.
    cell: vec2<u32>,
    level: u32,
    side: u32,
) -> f32 {
    let width = max(u32(inti_pages.world.w), 1u);
    let half = f32(width) * 0.5;
    let last = f32(page_texels) - 1.0;
    let corner = floor(texel - vec2<f32>(half));
    let frac = texel - vec2<f32>(half) - corner;
    var lit = 0.0;
    for (var y = 0u; y <= width; y = y + 1u) {
        var wy = 1.0;
        if y == 0u {
            wy = 1.0 - frac.y;
        } else if y == width {
            wy = frac.y;
        }
        for (var x = 0u; x <= width; x = x + 1u) {
            var wx = 1.0;
            if x == 0u {
                wx = 1.0 - frac.x;
            } else if x == width {
                wx = frac.x;
            }
            // 🔴 A tap that leaves this page is resolved through the
            // TABLE, not folded back onto the edge.
            //
            // Clamping is what shipped and its price is a lit band along
            // every page seam. The kernel is `W` texels wide, so a
            // receiver within `W/2` of an edge has taps that belong to
            // the neighbouring page — and whenever the shadow crosses a
            // seam the occluder's depth is exactly there. Clamped, those
            // taps read this page instead, find nothing, and the pixel
            // answers LIT with the page present, resident and correctly
            // drawn. The debug view calls that green: a real page whose
            // COMPARISON is wrong.
            //
            // A texel is centimetres at the fine levels and metres at
            // the coarse ones, so the band is a hairline near the camera
            // and a wedge further out.
            //
            // Unreal resolve every sample the same way —
            // `VirtualToPhysicalTexel` per tap inside `SampleBilinear`.
            // The neighbour may be absent, and then this falls back to
            // the clamp: no worse than before, and the level walk above
            // has already tried the coarser levels.
            let raw = corner + vec2<f32>(f32(x), f32(y));
            let outside = raw.x < 0.0 || raw.y < 0.0 || raw.x > last || raw.y > last;
            var tap = clamp(raw, vec2<f32>(0.0), vec2<f32>(last));
            var at = vec2<i32>(origin + tap);
            var tap_layer = layer;
            if outside && level != PAGE_UNLISTED {
                // Which page the tap fell into, in whole pages, and the
                // wrapped cell it lands on — the same toroidal grid
                // `sun_cell` keys by.
                let step = vec2<i32>(floor(raw / f32(page_texels)));
                let wide = i32(side);
                let neighbour = vec2<u32>(
                    (vec2<i32>(cell) + step + vec2<i32>(wide, wide)) % vec2<i32>(wide, wide),
                );
                let page = inti_pages.views.x * inti_pages.views.y
                    + inti_pages.space.w * inti_pages.space.x
                    + level * side * side
                    + neighbour.y * side
                    + neighbour.x;
                let found = inti_page_lookup(page);
                if found != PAGE_MISS {
                    let place = page_place(
                        found,
                        inti_pages.views.z,
                        inti_pages.pool.z,
                        page_texels,
                    );
                    tap = raw - vec2<f32>(step) * f32(page_texels);
                    at = vec2<i32>(vec2<f32>(place.xy) + tap);
                    tap_layer = i32(place.z);
                }
            }
            let stored = textureLoad(inti_page_atlas, at, tap_layer, 0);
            // 🔴 This tap's OWN depth, not the pixel's. The receiver is
            // a plane, so a tap `k` texels away looks at a part of it
            // sitting `dot(slope, k)` further along the light. Comparing
            // every tap against the depth under the PIXEL is what makes
            // a tilted surface shadow itself, and no scalar bias can
            // repair it because the error depends on which way the tap
            // moved. See `receiver_slope`.
            let here = receiver + dot(slope, tap - texel);
            // Reversed-Z: a LARGER stored depth is closer to the light,
            // so it is an occluder.
            let hit = select(1.0, 0.0, stored > here);
            lit = lit + hit * wx * wy;
        }
    }
    return lit / (f32(width) * f32(width));
}

/// The sun's shadow, out of the page pool.
///
/// # 🔴 It walks levels instead of recomputing which one was marked
///
/// The marking pass chose a level from the screen's pixel density, which
/// needs the camera's focal length and the render size. Reproducing that
/// arithmetic here would be a third copy of it, free to drift by a
/// rounding step — and a level off by one is a lookup that MISSES, which
/// reads as a shadow that disappears rather than as a shadow at the
/// wrong scale.
///
/// So it starts at the coarsest level that could possibly contain the
/// point and walks outward, taking the first page that is resident. Any
/// resident page containing the point holds correct depth, whatever
/// level marked it — the stored value is a distance along the sun's
/// axis, and that does not depend on how finely the page was diced.
/// Typically the first probe hits.
///
/// # 🔴 The bias is measured in TEXELS, because a clipmap's texel is not
/// one size
///
/// A cascade covers its whole range at one resolution, so a bias in
/// metres and a bias in texels are the same number twice and either
/// works. A clipmap is the opposite by construction: level 0's texel is
/// sub-millimetre and the last level's is metres across. One constant in
/// metres is therefore invisible at the far end of the chain and
/// enormous at the near end — and the near end is where an object meets
/// the ground.
///
/// So this does what `inti_sample_cascade_record` does, with the same
/// two constants: it moves the SAMPLED POSITION along the surface
/// normal by a multiple of the texel that is about to be read, instead
/// of adding to the depth it compares against. `INTI_NORMAL_BIAS`'s own
/// doc says why — *"moving along the surface instead of pushing depth is
/// what stops acne without detaching the shadow"* — and a detached
/// shadow is exactly what a depth-only bias produces where the gap
/// One probe of the sun's atlas at a world position: what the page holds
/// there, and what depth this position itself has.
///
/// Split out of `inti_page_shadow`'s walk so the march can ask the same
/// question per sample. Returns `x` the stored depth, `y` this
/// position's own depth in the same encoding, `z` 1 when a page was
/// found and `0` when the chain holds none.
fn inti_page_read(p: vec3<f32>, basis: mat3x3<f32>) -> vec3<f32> {
    let base = inti_pages.world.x;
    let span = inti_pages.world.y;
    let side = inti_pages.space.z;
    let page_texels = inti_pages.pool.w;

    let raw = sun_plane(p, basis) - sun_plane(inti_pages.eye.xyz, basis);
    let reach = max(abs(raw.x), abs(raw.y)) * 2.0;
    var level = sun_level(reach, base, side);

    for (; level < inti_pages.chain.x; level = level + 1u) {
        let along = dot(p - inti_pages.eye.xyz, basis[2])
            + sun_drift(inti_pages.eye.xyz, basis, base, side, level);
        let mine = 1.0 - (along + span) / (2.0 * span);

        let cell = sun_cell(p, inti_pages.eye.xyz, basis, base, side, level);
        let page = inti_pages.views.x * inti_pages.views.y
            + inti_pages.space.w * inti_pages.space.x
            + level * side * side
            + cell.y * side
            + cell.x;
        let slot = inti_page_lookup(page);
        if slot == PAGE_MISS {
            continue;
        }
        let rect = sun_page_rect(level, cell, inti_pages.eye.xyz, basis, base, side);
        let within = (sun_plane(p, basis) - rect.xy) / rect.z + vec2<f32>(0.5);
        let place = page_place(slot, inti_pages.views.z, inti_pages.pool.z, page_texels);
        let texel = clamp(
            floor(within * f32(page_texels)),
            vec2<f32>(0.0),
            vec2<f32>(f32(page_texels) - 1.0),
        );
        let at = vec2<i32>(vec2<f32>(place.xy) + texel);
        let stored = textureLoad(inti_page_atlas, at, i32(place.z), 0);
        return vec3<f32>(stored, mine, 1.0);
    }
    return vec3<f32>(0.0, 0.0, 0.0);
}

/// Rays cast over the sun's disc, and samples along each.
///
/// 🔴 A single ray is USELESS for a directional light, and that is the
/// whole reason the spread exists rather than being a soft-shadow
/// nicety. Stepping along the sun's own axis does not move the sample
/// in the sun's plane at all — `basis[2]` is perpendicular to the two
/// axes the page is addressed by — so every step of an unjittered march
/// reads the SAME texel at a different reference depth. The rays have to
/// open over the source's angular size for the march to see anything
/// the single tap did not.
const PAGE_RAYS: u32 = 4u;
const PAGE_STEPS: u32 = 8u;

/// Occlusion by marching the atlas instead of sampling one texel.
///
/// # 🔴 What a single tap cannot answer
///
/// The PCF reader asks "what is stored at the texel under this pixel",
/// so an occluder that did not land in that exact texel is not found and
/// the pixel is lit — a hole inside a shadow with the page present,
/// resident and correctly drawn. Widening the filter does not fix it:
/// every tap is still a comparison against ONE depth at ONE place, and
/// the taps that miss vote lit.
///
/// This asks a different question — *does anything block along this
/// ray* — and answers it with several samples per ray over several rays
/// spread across the sun's disc. An occluder missed by one sample is
/// found by another.
///
/// # The tolerance is measured, not chosen
///
/// Each step compares against how far the ray's OWN reference depth
/// moved since the previous step. On a surface facing the sun that
/// difference is tiny and the test is tight; on a grazing one it is
/// large and the test loosens by exactly as much as the geometry
/// demands. That is what makes the march need no bias constant at all,
/// and it is the mechanism a scalar bias was standing in for badly.
/// Unreal derive theirs the same way.
fn inti_page_march(
    world_position: vec3<f32>,
    normal: vec3<f32>,
    n_dot_l: f32,
    jitter: f32,
) -> f32 {
    let basis = sun_basis(inti_pages.sun.xyz);
    let base = inti_pages.world.x;
    let side = inti_pages.space.z;
    let page_texels = inti_pages.pool.w;
    // Along the sun's axis, towards it. `basis[2]` points the way the
    // light travels, so the ray runs against it.
    let to_light = -basis[2];

    // 🔴 How far a ray reaches, taken from the EXTENT of the clipmap
    // level this receiver lands in rather than from the orthographic
    // span.
    //
    // The span is 2000 metres. A ray that long put its very FIRST
    // sample sixty metres away — the steps are quadratic — so the
    // march never once looked near the surface it was shading, which
    // is where every occluder that matters is. The level's extent is
    // the right scale by construction: it is the quantity the page
    // addressing itself is built on, so the ray is short where the
    // texels are fine and long where they are coarse.
    let raw = sun_plane(world_position, basis) - sun_plane(inti_pages.eye.xyz, basis);
    let level = sun_level(max(abs(raw.x), abs(raw.y)) * 2.0, base, side);
    let extent = base * exp2(f32(level));
    let reach = extent;
    // One texel of that level — the unit every other bias here is in.
    let texel_world = extent / f32(side * page_texels);

    // The sun's angular radius, as the tangent of the half-angle. Zero
    // makes every ray identical and the march degenerate, so it has a
    // floor: a disc that small is a hard shadow either way.
    let spread = max(inti.sun_softness, 1e-3);

    // Off the surface by the same texel multiple the box reader uses,
    // so the first samples do not read the receiver itself.
    let start = world_position + normal * (texel_world * inti_pages.bias.x)
        + to_light * inti_pages.bias.y;

    var lit = 0.0;
    for (var r = 0u; r < PAGE_RAYS; r = r + 1u) {
        // Golden-angle spiral over the disc, rotated per pixel so the
        // pattern does not print itself onto flat ground.
        let t = (f32(r) + 0.5) / f32(PAGE_RAYS);
        let angle = f32(r) * 2.39996323 + jitter * 6.2831853;
        let radius = sqrt(t) * spread;
        let dir = normalize(
            to_light + basis[0] * (cos(angle) * radius) + basis[1] * (sin(angle) * radius),
        );

        var blocked = false;
        var previous = -1.0;
        for (var i = 1u; i <= PAGE_STEPS; i = i + 1u) {
            // Quadratic in the step index: dense near the receiver,
            // where contact shadows live and where a miss is most
            // visible, sparse further out.
            let f = f32(i) / f32(PAGE_STEPS);
            let at = start + dir * (f * f * reach);
            let read = inti_page_read(at, basis);
            if read.z == 0.0 {
                continue;
            }
            let reference = read.y;
            if previous >= 0.0 {
                // 🔴 The tolerance the geometry asks for, rather than a
                // constant: how far this ray's own depth moved in one
                // step. The 1.05 is Unreal's, and its reason is stated
                // there — without a little slack, surfaces are missed
                // to numeric precision and fully shadowed regions
                // sparkle.
                let tolerance = abs(reference - previous) * 1.05;
                // 🔴 Reversed-Z: a LARGER stored depth is NEARER the
                // light, so the occluder is the one whose stored depth
                // EXCEEDS the sample's own — not the other way round.
                //
                // Inverted, this marks blocked whenever the sample is
                // nearer the light than what is stored; every ray
                // marches towards the light, so its own depth climbs
                // past the ground it started from and every pixel ends
                // up reporting itself occluded. The frame came out
                // uniformly shadowed, with banding across every curved
                // surface where the steps landed.
                if read.x - reference > tolerance {
                    blocked = true;
                    break;
                }
            }
            previous = reference;
        }
        if !blocked {
            lit = lit + 1.0;
        }
    }
    return lit / f32(PAGE_RAYS);
}


/// between caster and receiver is smaller than the bias itself.
fn inti_page_shadow(
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_light: vec3<f32>,
    n_dot_l: f32,
) -> f32 {
    // Which reader answers, from `shadow_page_march`. See
    // `inti_page_march` for what the two ask differently.
    if inti_pages.layer.z != 0u {
        return inti_page_march(world_position, normal, n_dot_l, 0.0);
    }
    let basis = sun_basis(inti_pages.sun.xyz);

    let base = inti_pages.world.x;
    let span = inti_pages.world.y;
    let side = inti_pages.space.z;
    let page_texels = inti_pages.pool.w;

    // Containment is a floor on the level: a point outside a level's
    // extent has no page there to find. Mirrors `mark_sun`'s `contain`,
    // and it is measured from the UNOFFSET position on purpose — the
    // offset below is one texel wide and the boundary it would have to
    // cross is a whole page.
    let raw = sun_plane(world_position, basis) - sun_plane(inti_pages.eye.xyz, basis);
    let reach = max(abs(raw.x), abs(raw.y)) * 2.0;
    var level = sun_level(reach, base, side);

    for (; level < inti_pages.chain.x; level = level + 1u) {
        let extent = base * exp2(f32(level));
        // What one texel of THIS level covers, in metres: `side` pages
        // across the extent, `page_texels` texels across a page. This is
        // the clipmap's answer to `IntiCascade::texel_world_size`, and
        // the only reason the offset has to be computed inside the walk
        // rather than once before it.
        let texel_world = extent / f32(side * page_texels);
        // 🔴 CAPPED, and the cap is the whole point. The step is a
        // multiple of the TEXEL, and a clipmap texel is 0.1 mm at level
        // 0 and 5.12 m at level 16 — so the same constant is 0.0002 m
        // of offset at the finest level and 9.2 m at the coarsest. Nine
        // metres walks a receiver clean out of the volume its caster
        // shadows: the page is present, resident and correctly drawn,
        // and the comparison still answers LIT.
        //
        // ⚠️ `bias.z` of 0 means NO cap, which is what shipped and what
        // a project with no settings file still gets.
        var offset = texel_world * inti_pages.bias.x;
        if inti_pages.bias.z > 0.0 {
            offset = min(offset, inti_pages.bias.z);
        }
        let sampled = world_position
            + normal * offset
            + to_light * inti_pages.bias.y;

        // The plane is ABSOLUTE and the grid is snapped, so a texel's
        // footprint does not slide with the camera. See `sun_centre`.
        let centre = sun_centre(inti_pages.eye.xyz, basis, base, side, level);
        let along = dot(sampled - inti_pages.eye.xyz, basis[2])
            + sun_drift(inti_pages.eye.xyz, basis, base, side, level);
        // Reversed-Z along the sun's axis, matching `page_depth.wgsl`.
        // Nothing is added to it: the offset above already moved the
        // point towards the light, which is the depth half of the bias.
        let receiver = 1.0 - (along + span) / (2.0 * span);

        // The same absolute-world key the marking wrote. See `sun_cell`.
        let cell = sun_cell(sampled, inti_pages.eye.xyz, basis, base, side, level);
        // 🔴 The VIEW is the high part of the key. Two viewports over
        // one world are two clipmaps centred on two cameras, so the
        // same world position is a different page in each — and a
        // lookup without the view finds whichever camera marked last.
        let page = inti_pages.views.x * inti_pages.views.y
            + inti_pages.space.w * inti_pages.space.x
            + level * side * side
            + cell.y * side
            + cell.x;
        let slot = inti_page_lookup(page);
        if slot == PAGE_MISS {
            continue;
        }

        // Where the point sits inside its own page, in texels.
        let rect = sun_page_rect(level, cell, inti_pages.eye.xyz, basis, base, side);
        let within = (sun_plane(sampled, basis) - rect.xy) / rect.z + vec2<f32>(0.5);
        let place = page_place(slot, inti_pages.views.z, inti_pages.pool.z, page_texels);
        let origin = vec2<f32>(place.xy);
        let layer = i32(place.z);
        let texel = within * f32(page_texels);

        // 🔴 The receiver-plane gradient, in the texels of THIS level.
        // `bias.w` clamps it, and 0 disables the term entirely — which
        // is the A/B that says whether it is doing anything.
        let slope = receiver_slope(normal, basis, texel_world, span, inti_pages.bias.w);

        // Bilinear PCF, clamped inside the page — see
        // `inti_page_filter` for both halves of that sentence.
        return inti_page_filter(
            origin,
            layer,
            texel,
            receiver,
            slope,
            page_texels,
            cell,
            level,
            side,
        );
    }
    // No page anywhere in the chain. Lit, not shadowed: a point nobody
    // marked is a point the frame never looked at, and guessing dark
    // there would put shadow where no data exists.
    return 1.0;
}
/// A local light's shadow, out of the page pool.
///
/// # 🔴 The half that makes the pages visible
///
/// Rasterising a lamp's pages and never reading them is a pass that
/// costs and shows nothing — which is exactly what shipped one commit
/// ago. `inti_point_shadow` and `inti_spot_shadow` sampled the cube
/// atlas whatever the pages held, so a lamp past the cube budget
/// returned fully lit while its own pages sat drawn in the pool.
///
/// # Walking the chain, not computing the level
///
/// The MARKING picks a level from the texel a pixel wants, and that
/// number is a property of the frame the marking ran in — the reader has
/// no way back to it. So it walks: finest level first, taking the first
/// page that is resident. Whichever level the marking chose is the
/// coarsest one it could have chosen, so the walk finds it or something
/// finer, and never something coarser than the frame asked for.
///
/// `face` is the cube face the point lands on, except for a spot, which
/// writes one face the way `mark_local` assigns it.
fn inti_local_page_shadow(
    light: u32,
    is_spot: bool,
    light_position: vec3<f32>,
    // The spot's axis; unread for a point. A spot's one face is
    // aligned with it — see `spot_local`.
    light_direction: vec3<f32>,
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_light: vec3<f32>,
) -> f32 {
    let side0 = inti_pages.space.z;
    let page_texels = inti_pages.pool.w;
    let stride = inti_pages.space.x;
    let face_pages = inti_pages.space.y;
    // The chain stops where a whole level is one page. Mirrors
    // `PageConfig::levels`.
    let levels = u32(log2(f32(max(side0, 1u)))) + 1u;
    let view_base = inti_pages.views.x * inti_pages.views.y;

    // 🔴 Starts at the floor, not at zero. The marking cannot pick a
    // level below it, so the levels under it hold no pages for anybody —
    // walking them is three table lookups a pixel that can only miss.
    for (var level = local_level_floor(side0 * page_texels); level < levels; level = level + 1u) {
        let side = level_side_of(level, side0);
        let raw = world_position - light_position;
        let distance = max(length(raw), PAGE_NEAR);
        // A cube face is a 90-degree perspective, so at `distance` it
        // covers `2 * distance` across `side * page_texels` texels. The
        // same identity `page_level` inverts, and the reason the offset
        // is computed inside the walk rather than once before it.
        let texel_world = 2.0 * distance / f32(side * page_texels);
        // 🔴 The POINT pair, not the sun's. `INTI_POINT_DEPTH_BIAS` is
        // FOUR TIMES `INTI_DEPTH_BIAS` and its doc says exactly why: a
        // cube face is 90 degrees and its texels are coarse, so a lamp
        // needs more depth push than a cascade. Borrowing the sun's
        // prints a stair-stepped square on an empty floor under a lamp
        // — the floor shadowing itself — which is what this reader
        // shipped doing.
        let sampled = world_position
            + normal * (texel_world * INTI_POINT_NORMAL_BIAS)
            + to_light * INTI_POINT_DEPTH_BIAS;

        var offset = sampled - light_position;
        if is_spot {
            offset = spot_local(light_direction, offset);
        }
        let hit = cube_face(offset);
        let face = select(u32(hit.w), 0u, is_spot);
        let cell = vec2<u32>(
            clamp(hit.xy, vec2<f32>(0.0), vec2<f32>(0.99999)) * f32(side)
        );
        // 🔴 The VIEW is the high part of the key, the same as the sun's:
        // two viewports are two page sets and a lookup without it finds
        // whichever camera marked last.
        let page = view_base
            + light * stride
            + face * face_pages
            + local_level_base(level, side0, page_texels)
            + cell.y * side
            + cell.x;
        let slot = inti_page_lookup(page);
        if slot == PAGE_MISS {
            continue;
        }

        // Reversed-Z, and along the MAJOR AXIS — `page_depth.wgsl` stores
        // `PAGE_NEAR / major`, the same identity `GpuPointShadow`
        // documents for the cube path. A radial distance here would be
        // wrong by the ratio between the two, which is 1 at the centre
        // of a face and 1.73 at its corner: a shadow that is correct
        // straight ahead of the lamp and drifts towards every edge.
        let major = max(max(abs(offset.x), abs(offset.y)), abs(offset.z));
        let receiver = clamp(PAGE_NEAR / max(major, PAGE_NEAR), 0.0, 1.0);

        // Where the point sits inside its own cell, in texels.
        let step = 1.0 / f32(side);
        let low = vec2<f32>(cell) * step;
        let within = (hit.xy - low) / step;
        let place = page_place(slot, inti_pages.views.z, inti_pages.pool.z, page_texels);
        let origin = vec2<f32>(place.xy);
        let layer = i32(place.z);
        let texel = within * f32(page_texels);

        // 🔴 No gradient here YET, and the zero is deliberate rather
        // than an omission. A lamp's page is a PERSPECTIVE projection
        // storing `PAGE_NEAR / major`, so its receiver plane has a
        // different derivation than the sun's linear span — the same
        // repair, a different algebra. Passing zero is exactly today's
        // behaviour, so the sun's fix can be measured on its own.
        let slope = vec2<f32>(0.0);

        // Bilinear PCF, clamped inside the page — see
        // `inti_page_filter` for both halves of that sentence.
        // The lamps clamp: their pages are six faces of a chain, not
        // one wrapped plane, so a tap off an edge crosses a FACE and the
        // neighbour is not a step away on any grid this can index.
        return inti_page_filter(
            origin,
            layer,
            texel,
            receiver,
            slope,
            page_texels,
            vec2<u32>(0u),
            PAGE_UNLISTED,
            0u,
        );
    }
    // No page anywhere in the chain: lit, for the same reason the sun's
    // reader is. A point nobody marked is a point the frame never looked
    // at, and guessing dark there puts shadow where no data exists.
    return 1.0;
}

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
    // 🔴 The virtual shadow map REPLACES the cascades rather than
    // blending with them. Two techniques over one surface disagree at
    // their own boundaries, and the disagreement reads as a seam that
    // belongs to neither.
    if (inti_pages.sun.w > 0.5) {
        return inti_page_shadow(world_position, normal, to_light, n_dot_l);
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

// Everything about a shaded point that does not depend on which light
// is being summed. Built once per pixel, read once per light.
//
// It is a struct rather than eleven parameters because the debug views
// need to run the same per-light maths for a single light, and the only
// way to guarantee a view shows what the frame does is for both to call
// the same function. WGSL inlines it; nothing is copied per light.
struct IntiSurface {
    world_position: vec3<f32>,
    n: vec3<f32>,
    // Towards the camera.
    v: vec3<f32>,
    // The view vector mirrored about the normal — where a perfectly
    // smooth surface would be looking. Only the representative point
    // (#776) reads it, but it depends on nothing per-light, so it is
    // computed once per pixel here rather than once per light.
    r: vec3<f32>,
    n_dot_v: f32,
    diffuse_color: vec3<f32>,
    f0: vec3<f32>,
    perceptual: f32,
    // Linear roughness, `perceptual²`.
    a: f32,
    f_ab: vec2<f32>,
    // Distance along the view axis, which is what picks a cascade.
    //
    // Not the radial distance to the camera: that makes every cascade
    // boundary a sphere, so it crosses in the corners of the screen
    // before the centre and the split sweeps across the frame as the
    // camera turns. Projecting onto the forward axis makes the boundary
    // a plane, which is what the cascade fit assumes.
    view_depth: f32,
    // #804 — the instance's bits. Bit 0 is "receives shadows"; when it
    // is clear, `inti_light_contribution` skips the shadow fetch
    // outright rather than fetching and multiplying by one.
    flags: u32,
}

// `base_color` is linear albedo (sRGB textures are decoded by the
// sampler; `Material::base_color` is documented linear). `metallic`
// and `roughness` are the usual perceptual [0,1] scalars.
fn inti_surface(
    world_position: vec3<f32>,
    n: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    flags: u32,
) -> IntiSurface {
    let v = normalize(inti.camera_position - world_position);
    let n_dot_v = max(dot(n, v), 1e-4);
    let perceptual = clamp(roughness, INTI_MIN_PERCEPTUAL_ROUGHNESS, 1.0);

    var surf: IntiSurface;
    surf.world_position = world_position;
    surf.n = n;
    surf.v = v;
    surf.r = reflect(-v, n);
    surf.n_dot_v = n_dot_v;
    // Metals have no diffuse and take F0 from their albedo;
    // dielectrics reflect a flat 4% and keep all of theirs.
    surf.diffuse_color = base_color * (1.0 - metallic);
    surf.f0 = mix(vec3<f32>(0.04), base_color, metallic);
    surf.perceptual = perceptual;
    surf.a = perceptual * perceptual;
    surf.f_ab = inti_f_ab(perceptual, n_dot_v);
    surf.view_depth = dot(world_position - inti.camera_position, inti.camera_forward);
    surf.flags = flags;
    return surf;
}

// Karis 2013's representative point: the point on the light's sphere
// closest to the mirror ray, standing in for integrating the BRDF over
// the whole sphere. Returns that direction in `xyz` and the widened
// linear roughness in `w`.
//
// "Representative Point Area Lights", s2013_pbs_epic_notes_v2.pdf p14-16.
fn inti_representative_point(
    r: vec3<f32>,
    a: f32,
    offset: vec3<f32>,
    radius: f32,
    distance: f32,
) -> vec4<f32> {
    // 🔴 This max() is a FIX, not a guard against division by zero.
    // Bevy carries it for bevyengine/bevy#13318: "the point with the
    // smallest distance to the ray" is not merely imprecise but wrong
    // for a surface inside or touching the light's sphere, and without
    // the clamp such a surface shows a hard discontinuity. Anything
    // that looks like a redundant instruction here is that bug.
    let lt_f_dot_r = max(0.0001, dot(offset, r));
    let center_to_ray = lt_f_dot_r * r - offset;
    let closest = offset + center_to_ray * saturate(
        radius * inverseSqrt(dot(center_to_ray, center_to_ray)));
    // Karis p14. The 2 is hand-tuned against reference renders, not
    // derived.
    let a_prime = saturate(a + radius / (2.0 * max(distance, 1e-4)));
    return vec4<f32>(closest * inverseSqrt(dot(closest, closest)), a_prime);
}

// Bevy's amendment to Karis 2013: feeding the widened roughness
// straight into the BRDF makes smooth materials read too rough and too
// dim, so what the BRDF actually gets is a lerp between the original
// and the widened one, weighted by how rough the material already was.
// Their own comment names Linearly Transformed Cosines (#779) as the
// correct fix and this as the tuned stand-in.
fn inti_specular_fix_remap(a: f32) -> f32 {
    let inv_a_sq = (1.0 - a) * (1.0 - a);
    return 1.0 - inv_a_sq * inv_a_sq;
}

// One light's answer, with enough of the question left attached for the
// caller to decide which light mattered most (#845).
struct IntiLit {
    // What this light adds to the surface point.
    radiance: vec3<f32>,
    // The ceiling this light had on this point: irradiance faced head-on,
    // times the cosine. The same number the range cut and
    // `specular_floor` already read — a light's weight in one scalar.
    reach: f32,
    // Unit vector toward it, which is what a contact march needs.
    to_light: vec3<f32>,
}

// What one light adds to one surface point, shadows included. Zero when
// the surface faces away from it.
fn inti_light_contribution(
    surf: IntiSurface,
    light: IntiLight,
    // 🔴 Its index in `inti_lights`, which is the light's identity in a
    // page key. `IntiLight` does not carry it — the struct is 80 bytes
    // against a Rust mirror and its three spare scalars are load-bearing
    // padding, so the index travels as an argument rather than in a pad.
    index: u32,
    frag_coord: vec2<f32>,
) -> vec3<f32> {
    return inti_light_lit(surf, light, index, frag_coord, true).radiance;
}

// The same, with the contact march made optional and the light's weight
// reported back.
//
// 🔴 `march == false` is not "no contact shadows" — it is "not here".
// The caller marches once, for whichever light came back with the
// highest `reach`, and applies the result to that light's radiance
// alone. See `inti_shade`.
fn inti_light_lit(
    surf: IntiSurface,
    light: IntiLight,
    index: u32,
    frag_coord: vec2<f32>,
    march: bool,
) -> IntiLit {
    let s = inti_sample_light(light, surf.world_position);
    let n_dot_l = dot(surf.n, s.to_light);
    let nothing = IntiLit(vec3<f32>(0.0), 0.0, s.to_light);
    if (n_dot_l <= 0.0) {
        return nothing;
    }

    // The most this light can put on this surface: its irradiance, faced
    // head-on. `inti_sample_light` already computed it, so the test costs
    // a max and a compare.
    //
    // 🔴 Zero here is EXACT rather than merely small (#835).
    // `inti_distance_attenuation` saturates its window, so a fragment past
    // the light's range reads `irradiance == 0.0`, and the return at the
    // bottom of this function multiplies everything between here and there
    // by it — both BRDF layers, the shadow cube, the contact march — to
    // arrive at the value this line returns directly.
    //
    // The froxel is why such a light is in the loop at all: a cell accepts
    // any light whose bounding sphere touches its AABB, which is correct
    // and deliberately conservative. It leaves roughly 26 of the ~40 lights
    // in the busiest cell reaching no part of a given pixel (#820).
    let reach = max(max(s.irradiance.x, s.irradiance.y), s.irradiance.z) * n_dot_l;
    if (reach <= 0.0) {
        return nothing;
    }

    // The diffuse layer always answers to the light's centre.
    let h = normalize(s.to_light + surf.v);
    let l_dot_h = saturate(dot(s.to_light, h));
    let diffuse_term = inti_fd_burley(surf.perceptual, surf.n_dot_v, n_dot_l, l_dot_h);

    // The specular layer answers to a different direction as soon as
    // the light has a size (#776) — the sphere's closest point to the
    // mirror ray rather than its centre — and to a wider roughness.
    //
    // 🔴 It is also the expensive half of the model: GGX `D`,
    // height-correlated Smith `V`, Schlick `F`, the multiscatter fit and
    // the representative point, per light per pixel. With ~15 lights
    // reaching a pixel in a lit scene (#820), a light contributing a
    // fraction of the frame's exposure spends all of that on a highlight
    // nobody can see. #821 — under `specular_floor`, it pays for the
    // diffuse layer only.
    //
    // `specular_floor == 0.0` takes the branch every time, which is what
    // every frame did before this existed.
    var specular = vec3<f32>(0.0);
    var n_dot_l_spec = n_dot_l;
    // ⚠️ Fresnel stands in at normal incidence rather than being
    // dropped. `f` weights the diffuse layer below, so a skipped
    // specular that also skipped `f` would BRIGHTEN the surface — a
    // missing highlight is invisible, an over-lit dielectric is not.
    var f = surf.f0;
    // `reach` is the same value the range cut above already computed —
    // one light's ceiling on this surface, read twice for two different
    // questions. This one asks whether the highlight is worth its cost.
    if (reach >= inti.specular_floor) {
        var l_spec = s.to_light;
        var a_spec = surf.a;
        var l_dot_h_spec = l_dot_h;
        var h_spec = h;
        var spec_intensity = 1.0;
        var solid_angle = 0.0;
        // A directional light is excluded by kind and not only by its
        // radius: it has no position, so there is no distance for the
        // approximation to correct. A sun's angular size is a shadow
        // problem (#477), not this one.
        if (light.radius > 0.0 && light.kind != INTI_KIND_DIRECTIONAL) {
            let rep = inti_representative_point(
                surf.r, surf.a, s.offset, light.radius, s.distance);
            l_spec = rep.xyz;
            h_spec = normalize(l_spec + surf.v);
            n_dot_l_spec = saturate(dot(surf.n, l_spec));
            l_dot_h_spec = saturate(dot(l_spec, h_spec));
            // Spreading the same energy over a wider highlight must not
            // add any. Without this factor `radius` is a brightness knob.
            // Note it uses the RAW widened roughness, while the BRDF below
            // gets the remapped one — Bevy does both and they are not the
            // same number.
            let normalization = surf.a / max(rep.w, 1e-4);
            spec_intensity = normalization * normalization;
            a_spec = mix(surf.a, rep.w, inti_specular_fix_remap(surf.a));
            // Sphere visibility: at a grazing angle part of the sphere has
            // sunk below the horizon and cannot light this point at all.
            solid_angle = light.radius * light.radius
                / max(s.distance * s.distance, 1e-8);
        }

        let n_dot_h = saturate(dot(surf.n, h_spec));
        let d = inti_d_ggx(a_spec, n_dot_h);
        let vis = inti_v_smith_correlated(a_spec, surf.n_dot_v, n_dot_l_spec);
        f = inti_fresnel(surf.f0, l_dot_h_spec);
        specular = inti_specular_multiscatter(
            d * vis * f * spec_intensity, surf.f0, surf.f_ab);
        if (solid_angle > 0.0) {
            specular *= saturate(n_dot_l_spec / max(n_dot_l_spec + solid_angle, 1e-4));
        }
    }

    // Energy the specular layer reflected is energy the diffuse
    // layer underneath never receives. Bevy's forward path skips
    // this weighting; their path tracer does not, and it is the
    // path tracer that is right.
    let diffuse = (vec3<f32>(1.0) - f) * surf.diffuse_color * diffuse_term;

    // Each light kind reaches its own map: the sun's cascades (#476),
    // a spot's single projected layer (#777), a point's cube (#778).
    // #734's light textures are the remaining half of this.
    // #804 — a surface that receives no shadows never fetches. Not a
    // cheaper fetch: none. The cost this removes is per pixel *and* per
    // casting light, which is the product that makes lighting expensive
    // (#780 attacks the same product from the other side).
    var shadow = 1.0;
    if ((surf.flags & INTI_SURFACE_RECEIVES_SHADOWS) != 0u) {
        if (light.kind == INTI_KIND_DIRECTIONAL) {
            shadow = inti_shadow(surf.world_position, surf.n, s.to_light, surf.view_depth, n_dot_l);
        } else if (light.kind == INTI_KIND_POINT) {
            // 🔴 The page path is NOT gated on `shadow_slot`. That slot
            // is a cube-atlas index and there are 32 of them; a lamp
            // past the budget returns fully lit from the cube reader,
            // which is the exact ceiling the pages exist to remove. A
            // page-backed lamp needs no slot at all — its pages are
            // keyed by the light's own index.
            if (inti_pages.sun.w > 0.5) {
                shadow = inti_local_page_shadow(
                    index, false, light.position, light.direction,
                    surf.world_position, surf.n, s.to_light);
            } else if (light.shadow_slot != INTI_NO_SHADOW_SLOT) {
                // Six faces of one cube (#778).
                shadow = inti_point_shadow(
                    light.shadow_slot, surf.world_position, surf.n, s.to_light, light.position);
            }
        } else if (light.kind == INTI_KIND_SPOT) {
            if (inti_pages.sun.w > 0.5) {
                // One face, aligned with the spot's own axis.
                shadow = inti_local_page_shadow(
                    index, true, light.position, light.direction,
                    surf.world_position, surf.n, s.to_light);
            } else if (light.shadow_slot != INTI_NO_SHADOW_SLOT) {
                // A spot casts into a layer of the same array the
                // cascades use (#777). Along the cone axis, the same
                // measure Bevy takes: the radial distance would widen
                // the bias towards the edge of the cone, where the map
                // is not coarser.
                let axial = dot(light.direction, surf.world_position - light.position);
                shadow = inti_spot_shadow(
                    light.shadow_slot, surf.world_position, surf.n, s.to_light, n_dot_l, axial);
            }
        }

        // Contact shadows (#735) — the last few centimetres the cascades
        // cannot resolve, for any light kind, because a screen-space
        // march needs no shadow map and so has no reason to be the sun's
        // privilege. Skipped where the cascade already shadows this
        // point: multiplying two occlusions of the same occluder darkens
        // twice, and a march that finds nothing cannot brighten it back.
        if (march && (light.flags & INTI_LIGHT_CONTACT_SHADOWS) != 0u && shadow > 0.0) {
            shadow *= inti_contact_shadow(
                surf.world_position, surf.n, surf.v, s.to_light, frag_coord);
        }
    }

    // 🔴 The cosine is applied PER LAYER, not factored out: the
    // specular layer answers to its own direction and so to its own
    // N·L. Bevy writes the same line
    // (`diffuse * derived_input.NdotL + specular_light * specular_derived_input.NdotL`).
    // Factoring `n_dot_l` back out would silently undo half of #776 —
    // and would look identical until someone authored a radius.
    let radiance = (diffuse * n_dot_l + specular * n_dot_l_spec) * s.irradiance * shadow;
    // `reach` is reported only when this light may still be marched: a
    // light whose own shadow map already blocks it must not win the
    // dominance test and spend the frame's one march on a point that is
    // dark either way.
    let marchable = shadow > 0.0 && (light.flags & INTI_LIGHT_CONTACT_SHADOWS) != 0u;
    return IntiLit(radiance, select(0.0, reach, marchable), s.to_light);
}

/// A point light's shadow: one cube, six faces (#778).
///
/// # Why the largest axis and not the distance
///
/// The six faces align with the world axes and their frustum planes meet
/// at 45°, so the world-space depth stored for a fragment is the largest
/// absolute component of the vector to it — NOT its length. Bevy's
/// comment says exactly this. Using the Euclidean distance would scale
/// the bias by up to √3 toward the corners of a face, which is the same
/// class of mistake as #777's axial-vs-radial spot bias: it reads as a
/// bias that cannot be tuned rather than as a wrong formula.
fn inti_point_shadow(
    slot: u32,
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_light: vec3<f32>,
    light_position: vec3<f32>,
) -> f32 {
    if (slot >= inti.point_shadow_count) {
        return 1.0;
    }
    let record = inti.point_shadows[slot];

    let surface_to_light = light_position - world_position;
    let abs_to_light = abs(surface_to_light);
    let distance_to_light =
        max(abs_to_light.x, max(abs_to_light.y, abs_to_light.z));

    // The same two world-space offsets the cascades and the spots use,
    // with the texel size resolved to metres by this fragment's own
    // distance — the record carries an angle, not a length.
    let texel_world = record.texel_world_size * distance_to_light;
    let offset_position = world_position
        + normal * (texel_world * INTI_POINT_NORMAL_BIAS)
        + to_light * INTI_POINT_DEPTH_BIAS;

    let frag_ls = offset_position - light_position;
    let abs_ls = abs(frag_ls);
    let major = max(abs_ls.x, max(abs_ls.y, abs_ls.z));
    // The whole depth reconstruction, and it is one divide because the
    // faces were rendered with the engine's infinite reverse-Z
    // projection. Reversed-Z: nearer the light is GREATER, which is what
    // the comparison sampler's `Greater` expects.
    let depth = record.near / max(major, 1e-4);

    // 🔴 Cube maps are left-handed and this engine is not. The six face
    // directions are stored swapped on Z (see `FACE_DIRECTIONS`) and the
    // sampling direction is mirrored here. Fixing either half alone puts
    // the shadow of everything in front of a lamp behind it.
    let dir = frag_ls * vec3<f32>(1.0, 1.0, -1.0);
    return inti_filter_cube(dir, depth, slot, texel_world);
}

// A branchless orthonormal basis around `z_basis`, which must be unit.
//
// Duff et al. 2017, "Building an Orthonormal Basis, Revisited" — Bevy's
// `orthonormalize`. It exists because a cube map has no uv plane to
// offset a filter tap in: the taps have to move across the tangent plane
// of the sampling DIRECTION, and that plane has to be built per pixel
// without a branch on which axis is safest to cross with.
fn inti_orthonormalize(z_basis: vec3<f32>) -> mat3x3<f32> {
    let sign = select(-1.0, 1.0, z_basis.z >= 0.0);
    let a = -1.0 / (sign + z_basis.z);
    let b = z_basis.x * z_basis.y * a;
    return mat3x3<f32>(
        vec3<f32>(1.0 + sign * z_basis.x * z_basis.x * a, sign * b, -sign * z_basis.x),
        vec3<f32>(b, sign + z_basis.y * z_basis.y * a, -z_basis.y),
        z_basis,
    );
}

// How wide the cube filter is, in shadow texels.
//
// 🔴 Bevy's equivalent is a fixed `POINT_SHADOW_SCALE = 0.003` in
// direction units, which at their 1024² face works out to roughly one
// texel. Ours is expressed in texels instead of in angle so that it does
// not silently change meaning when the face size does — and it is wider
// than theirs on purpose, because these faces render at 512² and the
// engine's owner asked for a soft shadow rather than a detailed one. A
// point light is a lamp; nobody looks for the outline of a chair leg in
// what it casts.
const INTI_POINT_FILTER_TEXELS: f32 = 2.0;

// The eight standard D3D MSAA sample positions, and the coefficients of
// a zero-mean identity-covariance 2D Gaussian evaluated at them. The
// coefficients sum to 1, so the filter needs no normalisation.
//
// ⚠️ Not the Castano filter the cascades and spots use, and Bevy's own
// comment says why: Castano is a 2D Gaussian that leans on bilinear
// hardware to get nine taps out of four fetches, and **that trick does
// not exist for a cubemap**. Eight explicit taps is the replacement.
fn inti_filter_cube(
    dir: vec3<f32>,
    depth: f32,
    slot: u32,
    // 🔴 Already in METRES at this fragment's distance — the caller has
    // multiplied the record's angular texel by `distance_to_light`.
    // Multiplying by the distance again here made the filter radius grow
    // with the distance SQUARED, which is not a wider blur but a
    // gradient smeared across the whole floor. Caught in the smoke, in
    // one frame.
    texel_world: f32,
) -> f32 {
    let positions = array<vec2<f32>, 8>(
        vec2<f32>(0.125, -0.375),
        vec2<f32>(-0.125, 0.375),
        vec2<f32>(0.625, 0.125),
        vec2<f32>(-0.375, -0.625),
        vec2<f32>(-0.625, 0.625),
        vec2<f32>(-0.875, -0.125),
        vec2<f32>(0.375, 0.875),
        vec2<f32>(0.875, -0.875),
    );
    let coeffs = array<f32, 8>(
        0.157112,
        0.157112,
        0.138651,
        0.130251,
        0.114946,
        0.114946,
        0.107982,
        0.079001,
    );

    // The tangent plane of the sampling direction, scaled so one unit of
    // the pattern is one shadow texel at this fragment's distance. The
    // offsets are added to a direction vector whose length is that same
    // distance, so the two are in the same units and the filter covers a
    // constant number of texels wherever the fragment is.
    let basis = inti_orthonormalize(normalize(dir))
        * (texel_world * INTI_POINT_FILTER_TEXELS);

    var sum = 0.0;
    for (var i = 0; i < 8; i = i + 1) {
        let offset = positions[i].x * basis[0] + positions[i].y * basis[1];
        sum += coeffs[i] * textureSampleCompareLevel(
            inti_point_cubes, inti_shadow_sampler, dir + offset, i32(slot), depth);
    }
    return sum;
}

/// A spot light's shadow (#777).
///
/// A spot has one map and no splits, so there is nothing to pick by view
/// depth and nothing to blend at a boundary — everything else is the
/// cascade path, called on the spot's own record.
fn inti_spot_shadow(
    slot: u32,
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_light: vec3<f32>,
    n_dot_l: f32,
    distance_to_light: f32,
) -> f32 {
    if (slot >= inti.spot_shadow_count) {
        return 1.0;
    }
    // Bevy multiplies the normal bias by the distance to the light in
    // exactly this place, and it is the whole reason a perspective map
    // does not need a bias tuned per scene: the texel a fragment lands
    // in really is that much bigger the further out it is.
    return inti_sample_cascade_record(
        inti.spot_shadows[slot], world_position, normal, to_light, n_dot_l,
        max(distance_to_light, 0.0));
}

// The whole model, for one surface point.
//
// Returns linear HDR radiance. Exposure and the transfer function are
// applied by `inti_tonemap` separately, so a shadow pass or a debug
// overlay can consume the raw value.
// The running sum, plus the strongest light seen so far.
struct IntiAccum {
    radiance: vec3<f32>,
    // That light's own radiance, so the march can be applied to it and
    // to nothing else.
    brightest: vec3<f32>,
    reach: f32,
    to_light: vec3<f32>,
}

fn inti_accumulate(acc: IntiAccum, lit: IntiLit) -> IntiAccum {
    var out = acc;
    out.radiance += lit.radiance;
    // Strictly greater, so the first of two equals wins and the choice
    // does not depend on which order the froxel happened to list them.
    if (lit.reach > out.reach) {
        out.brightest = lit.radiance;
        out.reach = lit.reach;
        out.to_light = lit.to_light;
    }
    return out;
}

fn inti_merge(a: IntiAccum, b: IntiAccum) -> IntiAccum {
    var out = a;
    out.radiance += b.radiance;
    if (b.reach > out.reach) {
        out.brightest = b.brightest;
        out.reach = b.reach;
        out.to_light = b.to_light;
    }
    return out;
}

fn inti_shade(
    world_position: vec3<f32>,
    n: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    frag_coord: vec2<f32>,
    // #804 — the instance's bits, straight off `VertexOutput.flags`.
    flags: u32,
) -> vec3<f32> {
    let surf = inti_surface(world_position, n, base_color, metallic, roughness, flags);

    // 🔴 One march per PIXEL instead of one per light (#845).
    //
    // The march is linear in taps and had no cap of any kind: measured
    // on the OneXFly it cost 1.7 ms per step, and with ~14 lights
    // reaching a pixel that is the whole frame budget spent on contact.
    // Every one of those marches asks the same depth buffer about the
    // same point; only the direction differs.
    //
    // So the loop marches nothing, remembers which light lit the point
    // hardest, and one march is applied to that light's radiance
    // afterwards. What is lost is the contact of the second-brightest
    // lamp, which in a scene lit by fourteen was already diluted past
    // seeing — the same arithmetic that makes one light's shadow
    // invisible among many.
    var acc = IntiAccum(vec3<f32>(0.0), vec3<f32>(0.0), 0.0, vec3<f32>(0.0));
    let dominant = inti_contact_dominant_only();
    if (inti.clustered == 0u) {
        // No grid this frame: every light, for every pixel. What this
        // did before #780, and what a headless test or a path with no
        // camera matrices still does. Correct, and the reason the frame
        // cost pixels x lights.
        for (var i = 0u; i < inti.light_count; i = i + 1u) {
            acc = inti_accumulate(acc, inti_light_lit(
                surf, inti_lights[i], i, frag_coord, !dominant));
        }
    } else {
        // Directional lights are not in the grid: they reach every cell,
        // so a cell listing them would say nothing. They are the light
        // buffer's leading entries — see `ExtractedLights`.
        for (var i = 0u; i < inti.directional_count; i = i + 1u) {
            acc = inti_accumulate(acc, inti_light_lit(
                surf, inti_lights[i], i, frag_coord, !dominant));
        }
        acc = inti_merge(acc, inti_clustered_lights(surf, world_position, frag_coord, dominant));
    }

    var radiance = acc.radiance;
    if (dominant && acc.reach > 0.0) {
        // Subtracting what the march removes, rather than re-adding a
        // shaded copy: the winner's radiance is already inside the sum.
        let shadow = inti_contact_shadow(
            surf.world_position, surf.n, surf.v, acc.to_light, frag_coord);
        radiance -= acc.brightest * (1.0 - shadow);
    }

    radiance += inti_ambient(n, surf.diffuse_color, surf.f0, surf.f_ab);
    return radiance;
}


// The punctual lights of this fragment's froxel, and only those.
//
// 🔴 This is the whole point of #780. The loop bounds come from the
// cell's record instead of from the scene, so a lamp across the map
// costs this pixel nothing — not its falloff, and not the shadow map
// sample that used to come with it.
//
// Points and spots are walked as two consecutive ranges rather than one
// list with a type test inside it: the grid stores each type's indices
// contiguously precisely so that the test does not have to exist here.
fn inti_clustered_lights(
    surf: IntiSurface,
    world_position: vec3<f32>,
    frag_coord: vec2<f32>,
    dominant: bool,
) -> IntiAccum {
    let cell = inti_clusters[inti_cluster_of(world_position, frag_coord)];
    let points_end = cell.offset + cell.point_count;
    // Reflection probes, irradiance volumes and decals follow the spots
    // in the same record. Nothing reads them yet; when something does,
    // it reads its own range and this loop does not change.
    var spots_end = points_end + cell.spot_count;
    // `KOOCH_LIGHT_LIMIT`. Applied here rather than in the caller so
    // both shading paths inherit it from the one place the walk is
    // written: the experiment has to be runnable against the fragment
    // path, which is the one every earlier capture was taken with.
    if (inti.light_limit != 0u) {
        spots_end = min(spots_end, cell.offset + inti.light_limit);
    }

    var acc = IntiAccum(vec3<f32>(0.0), vec3<f32>(0.0), 0.0, vec3<f32>(0.0));
    for (var i = cell.offset; i < spots_end; i = i + 1u) {
        // 🔴 Clamped against the list's real length. A frame whose
        // lighting overflowed the index list leaves later cells pointing
        // past the end of it, and an out-of-range storage read is
        // undefined — a cell rendering under-lit is a bug someone can
        // see, which is the better failure.
        if (i >= inti.cluster_capacity) {
            break;
        }
        acc = inti_accumulate(acc, inti_light_lit(
            surf, inti_lights[inti_cluster_indices[i]], inti_cluster_indices[i], frag_coord, !dominant));
    }
    return acc;
}

// Which cell of the grid a fragment is in, in grid coordinates.
//
// Split out of `inti_cluster_of` for #824. A compute pass shading a tile
// reduces its threads' cells to one min/max block per axis, and a linear
// index cannot be reduced that way: two pixels one z-slice apart differ
// by 1 in `z` and by nothing else, while their linear indices differ by
// an amount that says nothing about how far apart the cells are.
fn inti_cluster_cell(world_position: vec3<f32>, frag_coord: vec2<f32>) -> vec3<u32> {
    let view_z = dot(inti.view_z_row, vec4<f32>(world_position, 1.0));
    let xy = vec2<u32>(floor(frag_coord * inti.cluster_factors.xy));
    // Mirrors `cluster_z_slice` in `cluster_common.wgsl` and
    // `ClusterGrid::z_slice` in Rust. Three copies of four operations,
    // because a fragment that disagrees with the grid reads a cell the
    // grid never wrote for it.
    let slice = log(-view_z) * inti.cluster_factors.z - inti.cluster_factors.w + 1.0;
    let z = min(u32(max(slice, 0.0)), inti.cluster_dimensions.z - 1u);
    return clamp(
        vec3<u32>(xy, z),
        vec3<u32>(0u),
        inti.cluster_dimensions.xyz - vec3<u32>(1u));
}

// That cell's index into `inti_clusters`.
fn inti_cluster_index(cell: vec3<u32>) -> u32 {
    return min(
        (cell.y * inti.cluster_dimensions.x + cell.x) * inti.cluster_dimensions.z + cell.z,
        inti.cluster_dimensions.w - 1u);
}

// Which cell of the grid a fragment is in.
fn inti_cluster_of(world_position: vec3<f32>, frag_coord: vec2<f32>) -> u32 {
    return inti_cluster_index(inti_cluster_cell(world_position, frag_coord));
}

// The tonemap lives in `inti_tonemap.wgsl`, concatenated ahead of this
// file — see `INTI_TONEMAP`. It is shared with the standalone tonemap
// pass the compute shading path feeds (#732), which has an HDR texture
// and no lighting bindings, so the operator cannot read `inti` directly.
fn inti_tonemap(radiance: vec3<f32>) -> vec3<f32> {
    return inti_tonemap_with(radiance, inti.exposure);
}

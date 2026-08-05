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
    _pad0: f32,
    _pad1: f32,
}

// Per-frame lighting constants. `camera_position` lives here rather
// than in the shared camera UBO because that UBO is pinned at 64 B by
// two bind-group layouts, and widening it would ripple through paths
// this issue has no business touching.
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
}

@group({{INTI_GROUP}}) @binding(0) var<uniform> inti: IntiFrame;
// Always at least one element: wgpu rejects a zero-sized storage
// binding, so an unlit scene binds a one-element buffer with
// `light_count == 0` rather than needing a second pipeline.
@group({{INTI_GROUP}}) @binding(1) var<storage, read> inti_lights: array<IntiLight>;

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
) -> vec3<f32> {
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

        radiance += (diffuse + specular) * s.irradiance * n_dot_l;
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

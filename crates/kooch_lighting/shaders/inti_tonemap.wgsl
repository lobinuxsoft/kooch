// inti_tonemap.wgsl — HDR radiance to display-referred colour.
//
// Split out of `inti_pbr.wgsl` for #732. Two passes now apply it: the
// fragment shading path, which still tonemaps inline and has the whole
// Inti uniform to read `exposure` from, and the standalone tonemap pass
// the compute path feeds, which has an HDR texture and nothing else.
//
// 🔴 The split exists so there is exactly ONE copy of the operator.
// `compute_shading_parity` asserts the two shading paths agree to within
// one 255th, and two hand-kept copies of an ACES curve is precisely how
// that starts failing somewhere nobody thinks to look. Everything here
// takes `exposure` as an argument rather than reading `inti`, which is
// what lets a pass with no lighting bindings concatenate it.

// Narkowicz 2015 filmic approximation. Provisional: #254 owns the real
// tonemapper and the auto exposure that lets a sunlit surface and a
// planet's night side coexist in one frame.
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
// deliberately NON-sRGB surface format, so nothing downstream applies
// this curve and a frame that skipped it renders visibly dark.
fn inti_linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let cutoff = c < vec3<f32>(0.0031308);
    let low = c * 12.92;
    let high = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, cutoff);
}

// HDR radiance → the 8-bit value the surface expects.
fn inti_tonemap_with(radiance: vec3<f32>, exposure: f32) -> vec3<f32> {
    return inti_linear_to_srgb(inti_aces(radiance * exposure));
}

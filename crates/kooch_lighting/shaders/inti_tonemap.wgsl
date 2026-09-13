// HDR radiance → display colour, split from `inti_pbr.wgsl` (#732) so the fragment path and the
// standalone pass share ONE operator — `compute_shading_parity` holds them within 1/255. `exposure`
// is an argument, not `inti`.

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

// Linear → sRGB. 🔴 Not redundant: `GpuContext` picks a non-sRGB surface, so skipping it renders
// dark.
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

// tonemap.wgsl — HDR radiance to a display-referred image (#732).
//
// The pass that exists because temporal anti-aliasing has to run on
// linear radiance. See `HDR_COLOR_FORMAT` for why the tonemap could not
// stay inside the shading shader once TAA was on the roadmap.
//
// `inti_tonemap` is CONCATENATED from Inti, not reimplemented here: the
// fragment shading path still tonemaps inline, and two copies of an
// operator that must agree to within one 255th is how a parity test
// starts failing for a reason nobody can find.

struct TonemapUniforms {
    // 0 passes the colour through untouched. The debug views produce
    // display-ready colour — a cluster index, a LOD level, a normal —
    // and tonemapping a false-colour legend turns a readable ramp into
    // a washed-out one.
    enabled: u32,
    exposure: f32,
    // Source-over-target pixel ratio; 1 when they match, the render
    // scale when a debug view skipped the upscaler. NEAREST on purpose:
    // a false-colour legend must not blend across its own edges.
    scale: vec2<f32>,
}

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> tonemap: TonemapUniforms;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// The usual three-vertex cover. No vertex buffer, no index buffer.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varyings {
    var out: Varyings;
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    out.uv = uv;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: Varyings) -> @location(0) vec4<f32> {
    // 🔴 `textureLoad` on the integer pixel, not a filtered sample. The
    // source and destination are the same size and a linear tap would
    // read two texels wherever the sampler's rounding disagrees with the
    // rasteriser's, which shows up as a half-pixel blur that survives
    // every later pass.
    let texel = vec2<i32>(in.position.xy * tonemap.scale);
    let hdr = textureLoad(hdr_tex, texel, 0);
    if (tonemap.enabled == 0u) {
        return hdr;
    }
    // Alpha is coverage, not opacity: it is what the tests read to tell
    // a shaded pixel from the background, so it passes through.
    return vec4<f32>(inti_tonemap_with(hdr.rgb, tonemap.exposure), hdr.a);
}

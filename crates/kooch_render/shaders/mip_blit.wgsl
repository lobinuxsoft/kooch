// mip_blit.wgsl — one level of a mip chain, from the level above it.
//
// A fullscreen triangle over mip N, sampling mip N-1 with a linear
// filter. At exactly half the size every output texel lands on the
// corner shared by four input texels, so one bilinear tap IS the 2x2 box
// average — the hardware does the four reads and the divide.
//
// # 🔴 Why this is a render pass and not a compute shader
//
// The chain has to be averaged in LINEAR light, and the textures that
// need it most are `Rgba8UnormSrgb` albedo maps. Sampling an sRGB view
// decodes to linear and writing to an sRGB attachment re-encodes, both
// in fixed-function hardware and both exact. A compute shader would
// have to write through a storage binding, sRGB storage textures do not
// exist, and the transfer function would come back as two `pow` calls in
// WGSL that are wrong the day someone changes the format. Averaging
// gamma-encoded bytes directly is the classic mip bug: a checkerboard
// mips down to 128 instead of 188 and every distant surface reads darker
// than it should.

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

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
    return textureSample(source, source_sampler, in.uv);
}

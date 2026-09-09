// meshlet_blit.wgsl — composes the meshlet stage's Rgba8Unorm color
// texture onto an arbitrary RENDER_ATTACHMENT (typically the editor
// ViewportTarget's Bgra8Unorm or the swapchain surface's color view).
//
// Triangle-strip cover: 3 vertices generate a full-screen triangle
// without any vertex/index buffer. Sampling is bilinear so resizing
// the source vs target stays visually clean.

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@group(0) @binding(0) var src_color: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var src_depth: texture_depth_2d;
// Nearest: a depth between two surfaces is a value neither of them has.
@group(0) @binding(3) var depth_sampler: sampler;

@vertex
fn vs_blit(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    // Big covering triangle:
    //   v0 = (-1, -1)
    //   v1 = ( 3, -1)
    //   v2 = (-1,  3)
    let x = f32((vertex_index & 1u) << 2u) - 1.0;
    let y = f32((vertex_index & 2u) << 1u) - 1.0;
    var out: VsOut;
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);
    // UV in [0,1] across the whole quad. Y flipped so source's row-0
    // lands at the top of the destination.
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

struct Blitted {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fs_blit(input: VsOut) -> Blitted {
    var out: Blitted;
    out.color = textureSample(src_color, src_sampler, input.uv);
    // The stage's depth carried over with its colour. Anything drawn
    // into the destination afterwards — grid, gizmos, transparents —
    // tests against real geometry rather than an empty buffer.
    out.depth = textureSampleLevel(src_depth, depth_sampler, input.uv, 0);
    return out;
}

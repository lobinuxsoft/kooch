// material_frame_post.wgsl — the frame a post-process shader runs inside (#1201).
//
// 🔴 It hands the same `SurfaceInput` the scene's surfaces get, so every graph node keeps working:
// `uv` is the screen in 0..1, the world fields are zero, and `sample_scene` is the colour the
// camera produced. Entry points: `vs_fullscreen`, `fs_post`.

// What the surface contract reads off `screen`.
struct ScreenUniforms {
    material_id: u32,
    // A post-process samples at screen size, so the mip bias is 1.
    mip_bias_scale: f32,
    time: f32,
    _pad: u32,
}

// The contract reaches for `inti.camera_position`; a post-process has a camera behind it.
struct IntiFrame {
    camera_position: vec3<f32>,
    _pad: f32,
}

struct PostUniforms {
    // Pixels, so a shader can work in them rather than in uv.
    resolution: vec2<f32>,
    // How much of the effect mixes over what it read, 0..1 (#1209).
    weight: f32,
    _pad: f32,
}

@group(0) @binding(0) var scene_color: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> screen: ScreenUniforms;
@group(0) @binding(3) var<uniform> inti: IntiFrame;
@group(0) @binding(4) var<uniform> post: PostUniforms;

// What the prelude's `surface_input` takes. A post-process has no geometry, so the world fields are
// zero and only `uv` carries meaning.
struct VertexOutput {
    world_position: vec3<f32>,
    world_normal: vec3<f32>,
    uv: vec2<f32>,
    ddx_uv: vec2<f32>,
    ddy_uv: vec2<f32>,
    world_tangent: vec4<f32>,
    material_id: u32,
    flags: u32,
}

/// The frame the camera produced, at `uv`. What a post-process is for.
fn sample_scene(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(scene_color, scene_sampler, uv, 0.0);
}

/// Size of the frame, in pixels.
fn scene_size() -> vec2<f32> {
    return post.resolution;
}

struct PostVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> PostVertex {
    var out: PostVertex;
    let x = f32((vertex_index & 1u) << 2u) - 1.0;
    let y = f32((vertex_index & 2u) << 1u) - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    // y flipped: clip space climbs, a texture's rows descend.
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@fragment
fn fs_post(in: PostVertex) -> @location(0) vec4<f32> {
    var surf: VertexOutput;
    surf.uv = in.uv;
    // Screen-space derivatives are honest here: this is an ordinary full-screen raster.
    surf.ddx_uv = dpdx(in.uv);
    surf.ddy_uv = dpdy(in.uv);
    surf.world_normal = vec3<f32>(0.0, 0.0, 1.0);
    surf.world_tangent = vec4<f32>(1.0, 0.0, 0.0, 1.0);
    surf.material_id = screen.material_id;
    surf.flags = 0u;

    let effect = post_process(surface_input(surf, in.position.xy));
    // Blended here rather than in each shader, so every effect has a weight without asking for one.
    return mix(sample_scene(in.uv), effect, post.weight);
}

// material_frame_sky.wgsl — the frame a sky shader runs inside (#1900).
//
// 🔴 It hands the same `SurfaceInput` the scene's surfaces get, so every graph node keeps working:
// `world_normal` is the direction looked at, `world_position` one unit along it from the camera,
// and `uv` that direction's longitude and latitude in 0..1, so the 2D noise nodes wrap the sky.
// Entry points: `vs_fullscreen`, `fs_sky`.

struct ScreenUniforms {
    material_id: u32,
    mip_bias_scale: f32,
    time: f32,
    _pad: u32,
}

struct IntiFrame {
    camera_position: vec3<f32>,
    _pad: f32,
}

struct SkyFrame {
    // Clip space back to a world direction; translation left out, the sky has no position.
    inverse_view_projection: mat4x4<f32>,
}

@group(0) @binding(2) var<uniform> screen: ScreenUniforms;
@group(0) @binding(3) var<uniform> inti: IntiFrame;
@group(0) @binding(4) var<uniform> frame: SkyFrame;

struct VertexOutput {
    world_position: vec3<f32>,
    world_normal: vec3<f32>,
    uv: vec2<f32>,
    ddx_uv: vec2<f32>,
    ddy_uv: vec2<f32>,
    world_tangent: vec4<f32>,
    material_id: u32,
    flags: u32,
    layers: u32,
}

// Nothing has been drawn under a sky: the graph's Scene Color reads black here.
fn sample_scene(uv: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(0.0);
}

struct SkyVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> SkyVertex {
    var out: SkyVertex;
    let x = f32((vertex_index & 1u) << 2u) - 1.0;
    let y = f32((vertex_index & 2u) << 1u) - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.ndc = vec2<f32>(x, y);
    return out;
}

// The step across the longitude seam is a whole turn; taking it out keeps a texture's mip honest.
fn wrapped(d: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(d.x - round(d.x), d.y);
}

@fragment
fn fs_sky(in: SkyVertex) -> @location(0) vec4<f32> {
    let far = frame.inverse_view_projection * vec4<f32>(in.ndc, 0.5, 1.0);
    let dir = normalize(far.xyz / far.w);
    let uv = vec2<f32>(
        atan2(dir.x, -dir.z) * 0.15915494 + 0.5,
        acos(clamp(dir.y, -1.0, 1.0)) * 0.31830989,
    );

    var surf: VertexOutput;
    surf.world_position = inti.camera_position + dir;
    surf.world_normal = dir;
    surf.uv = uv;
    surf.ddx_uv = wrapped(dpdx(uv));
    surf.ddy_uv = wrapped(dpdy(uv));
    surf.world_tangent = vec4<f32>(1.0, 0.0, 0.0, 1.0);
    surf.material_id = screen.material_id;
    surf.flags = 0u;
    surf.layers = 0xffffffffu;
    return vec4<f32>(sky(surface_input(surf, in.position.xy)).rgb, 1.0);
}

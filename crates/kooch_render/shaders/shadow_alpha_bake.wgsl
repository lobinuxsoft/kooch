// shadow_alpha_bake.wgsl — one transparent material's coverage over its uv square, for the shadow
// rasters to read (#1224). The same `surface` the scene runs, asked at uv and time only: a shadow
// has no viewer to be seen from, so the rest of `SurfaceInput` is a plain upward surface.

struct ScreenUniforms {
    material_id: u32,
    mip_bias_scale: f32,
    time: f32,
    _pad: u32,
}

// Only `camera_position` is read by the contract, but the name has to be `inti`.
struct IntiFrame {
    camera_position: vec3<f32>,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> screen: ScreenUniforms;
@group(0) @binding(1) var<uniform> inti: IntiFrame;

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

const BAKE_SIDE: f32 = 128.0;

@vertex
fn vs_bake(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vertex_index & 1u) << 2u) - 1.0;
    let y = f32((vertex_index & 2u) << 1u) - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_bake(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    var surf: VertexOutput;
    surf.world_position = vec3<f32>(0.0);
    surf.world_normal = vec3<f32>(0.0, 1.0, 0.0);
    surf.world_tangent = vec4<f32>(1.0, 0.0, 0.0, 1.0);
    surf.uv = position.xy / BAKE_SIDE;
    surf.ddx_uv = vec2<f32>(1.0 / BAKE_SIDE, 0.0);
    surf.ddy_uv = vec2<f32>(0.0, 1.0 / BAKE_SIDE);
    surf.material_id = screen.material_id;
    surf.flags = 0u;
    let shaded = surface(surface_input(surf, position.xy));
    return vec4<f32>(clamp(shaded.alpha, 0.0, 1.0));
}

// Mesh pass — vertex transform + world-normal-as-color fragment.
//
// Group 0: camera (view_proj).
// Group 1: model (per-draw, dynamic uniform offset, 64 bytes per slot).
//
// Output: rgb = world_normal * 0.5 + 0.5 (classic normal-map debug look).
// No materials / no lighting yet — that lands in #130.

struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

struct ModelUniforms {
    model: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(1) @binding(0) var<uniform> model: ModelUniforms;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
}

@vertex
fn vs_main(input: VsIn) -> VsOut {
    var out: VsOut;
    let world_pos = model.model * vec4<f32>(input.position, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    // Approximation: assumes uniform scale. Non-uniform scale would need
    // transpose(inverse(model)) for correct normals — covered when #130
    // lands a materials path that needs accurate shading.
    out.world_normal = (model.model * vec4<f32>(input.normal, 0.0)).xyz;
    return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(input.world_normal);
    return vec4<f32>(n * 0.5 + 0.5, 1.0);
}

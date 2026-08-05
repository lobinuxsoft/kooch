// Gizmo line shader — quad-line rendering with screen-space thickness.
//
// Each line segment is rasterized as a 4-vertex quad (2 triangles).
// Per-vertex inputs carry the line's two endpoints, the vertex's side
// (+1/-1), and the thickness in physical pixels. The vertex shader
// projects both endpoints to clip space, computes the line direction
// in pixel space, and offsets the vertex perpendicular to the line by
// `thickness` pixels.
//
// Algorithm reference: Bevy `bevy_gizmos`'s `lines.wgsl` and the
// general "expand quad in screen space" technique used by every modern
// editor for thick line rendering on top of `wgpu` / Vulkan / DX12
// (which lack arbitrary line width).

struct CameraUniforms {
    view_proj: mat4x4<f32>,
    viewport_size: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) other_position: vec3<f32>,
    @location(3) side: f32,
    @location(4) thickness: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let pos_clip = camera.view_proj * vec4<f32>(input.position, 1.0);
    let other_clip = camera.view_proj * vec4<f32>(input.other_position, 1.0);

    // Convert to pixel space (NDC * viewport / 2). Using pixel space
    // makes the perpendicular offset trivially in the same units as
    // `thickness`.
    let pos_pixel = (pos_clip.xy / pos_clip.w) * camera.viewport_size * 0.5;
    let other_pixel = (other_clip.xy / other_clip.w) * camera.viewport_size * 0.5;

    let line_pixel = other_pixel - pos_pixel;
    let len = length(line_pixel);
    var dir = vec2<f32>(0.0, 1.0);
    if (len > 0.001) {
        dir = line_pixel / len;
    }
    let perp = vec2<f32>(-dir.y, dir.x);

    let offset_pixel = perp * input.thickness * input.side;
    let final_pixel = pos_pixel + offset_pixel;

    // Back to NDC, then re-multiply by w so the perspective divide
    // restores the correct screen-space position.
    let final_ndc = final_pixel / (camera.viewport_size * 0.5);

    var output: VertexOutput;
    output.clip_position = vec4<f32>(final_ndc * pos_clip.w, pos_clip.z, pos_clip.w);
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color, 1.0);
}

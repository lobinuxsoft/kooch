// Mesh gizmo shader — unlit alpha-blended triangles with shader-side
// edge highlighting.
//
// Per-vertex `edge_uv` lets the fragment shader detect proximity to a
// face perimeter: distance to closest of the four UV edges (u=0, u=1,
// v=0, v=1) is computed in screen space using `fwidth`, then a smooth
// step gives a crisp constant-pixel-width outline. Outline pixels keep
// the vertex color but force alpha to 1.0; interior pixels keep the
// vertex alpha so fills stay translucent.
//
// Geometry that wants no edges (e.g., the future filled cube) passes
// `edge_uv = (0.5, 0.5)` for every vertex so the fragment is always
// interior.
//
// Pipeline draws with `PrimitiveTopology::TriangleList`, depth-test
// `Always`, depth-write disabled — same always-on-top behavior as the
// line pipeline.

struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) edge_uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) edge_uv: vec2<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = camera.view_proj * vec4<f32>(input.position, 1.0);
    output.color = input.color;
    output.edge_uv = input.edge_uv;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Distance (in UV units) to the closest of the 4 edges.
    let edge_dist = min(
        min(input.edge_uv.x, 1.0 - input.edge_uv.x),
        min(input.edge_uv.y, 1.0 - input.edge_uv.y),
    );
    // Pixel-aware band width via screen-space derivatives so the
    // outline stays ~1.5 pixels wide at any camera distance.
    let band = fwidth(edge_dist) * 1.5;
    // 0 inside the face, 1 right at the edge.
    let edge_factor = 1.0 - smoothstep(0.0, band, edge_dist);
    // Mix between fill alpha and full alpha at edges.
    let alpha = mix(input.color.a, 1.0, edge_factor);
    return vec4<f32>(input.color.rgb, alpha);
}

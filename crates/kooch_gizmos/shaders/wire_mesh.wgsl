// Persistent wire meshes (#2030): triangles stay on the GPU, one draw over all of them. Each vertex
// fetches its corner by `vertex_index`, so the barycentric edge needs no extra vertex data.

struct Camera {
    view_proj: mat4x4<f32>,
}

struct Mesh {
    position: vec3<f32>,
    // rgba8; alpha 0 is a freed slot and draws nothing.
    color: u32,
    rotation: vec4<f32>,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> positions: array<f32>;
@group(0) @binding(2) var<storage, read> corners: array<u32>;
@group(0) @binding(3) var<storage, read> owners: array<u32>;
@group(0) @binding(4) var<storage, read> meshes: array<Mesh>;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) bary: vec3<f32>,
}

fn rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Out {
    var out: Out;
    let mesh = meshes[owners[index / 3u]];
    let color = unpack4x8unorm(mesh.color);
    let at = corners[index] * 3u;
    let local = vec3<f32>(positions[at], positions[at + 1u], positions[at + 2u]);
    let world = mesh.position + rotate(mesh.rotation, local);
    // A freed slot collapses outside the clip volume.
    out.clip = select(camera.view_proj * vec4<f32>(world, 1.0), vec4<f32>(0.0, 0.0, 2.0, 1.0), color.a == 0.0);
    out.color = color;
    let corner = index % 3u;
    out.bary = vec3<f32>(f32(corner == 0u), f32(corner == 1u), f32(corner == 2u));
    return out;
}

@fragment
fn fs_main(input: Out) -> @location(0) vec4<f32> {
    let edge = min(min(input.bary.x, input.bary.y), input.bary.z);
    let band = fwidth(edge) * 1.25;
    let line = 1.0 - smoothstep(0.0, band, edge);
    if line < 0.01 {
        discard;
    }
    return vec4<f32>(input.color.rgb, input.color.a * line);
}

// meshlet_vbuf.wgsl — meshlet rasterizer for the visibility-buffer
// path. Identical vertex routing to meshlet_main.wgsl, but the
// fragment writes a packed (meshlet_id, triangle_id) into a single
// R32Uint target instead of a color.
//
// Packing scheme:
//   bit  0..7    triangle index inside the meshlet (0..123, fits 7 bits)
//   bit  7..32   meshlet id + 1                    (25 bits → 33M meshlets)
// Encoded value 0 means "background" (cleared by the render pass);
// the +1 offset on meshlet_id keeps real meshlets distinguishable.

struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

struct ModelUniforms {
    model: mat4x4<f32>,
}

struct MeshVertexStored {
    position: array<f32, 3>,
    normal: array<f32, 3>,
    uv: array<f32, 2>,
}

struct MeshletDescriptor {
    vertex_offset: u32,
    triangle_offset: u32,
    vertex_count: u32,
    triangle_count: u32,
    aabb_min: vec3<f32>,
    _pad0: u32,
    aabb_max: vec3<f32>,
    _pad1: u32,
    bounds_center: vec3<f32>,
    bounding_radius: f32,
    cone_apex: vec3<f32>,
    cone_cutoff: f32,
    cone_axis: vec3<f32>,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<uniform> model: ModelUniforms;

@group(1) @binding(0) var<storage, read> vertices: array<MeshVertexStored>;
@group(1) @binding(1) var<storage, read> meshlet_vertices: array<u32>;
@group(1) @binding(2) var<storage, read> meshlet_triangles: array<u32>;
@group(1) @binding(3) var<storage, read> descriptors: array<MeshletDescriptor>;

@group(2) @binding(0) var<storage, read> visible_meshlets: array<u32>;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) @interpolate(flat) packed_id: u32,
}

fn fetch_local_vertex_index(byte_offset: u32) -> u32 {
    let word_idx = byte_offset / 4u;
    let byte_in_word = byte_offset & 3u;
    let packed = meshlet_triangles[word_idx];
    return (packed >> (byte_in_word * 8u)) & 0xffu;
}

@vertex
fn vs_vbuf(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VsOut {
    let meshlet_id = visible_meshlets[instance_index];
    let desc = descriptors[meshlet_id];

    let triangle_idx = vertex_index / 3u;
    let corner_idx = vertex_index % 3u;

    var out: VsOut;
    if (triangle_idx >= desc.triangle_count) {
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.packed_id = 0u;
        return out;
    }

    let byte_offset = desc.triangle_offset + triangle_idx * 3u + corner_idx;
    let local_vertex_idx = fetch_local_vertex_index(byte_offset);
    let global_vertex_idx = meshlet_vertices[desc.vertex_offset + local_vertex_idx];
    let v = vertices[global_vertex_idx];

    let pos = vec3<f32>(v.position[0], v.position[1], v.position[2]);
    let world_pos = model.model * vec4<f32>(pos, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    out.packed_id = ((meshlet_id + 1u) << 7u) | (triangle_idx & 0x7Fu);
    return out;
}

@fragment
fn fs_vbuf(input: VsOut) -> @location(0) u32 {
    return input.packed_id;
}

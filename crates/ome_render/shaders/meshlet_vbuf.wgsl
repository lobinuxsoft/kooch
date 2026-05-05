// meshlet_vbuf.wgsl — meshlet rasterizer for the visibility-buffer
// path. Identical vertex routing to meshlet_main.wgsl, but the
// fragment writes a packed integer into a single R32Uint target
// instead of a color.
//
// Two entry-point pairs share this file:
//
// (vs_vbuf, fs_vbuf) — single-mesh path (Phase 1.D):
//   Packing:
//     bit  0..7    triangle index inside the meshlet  (7 bits)
//     bit  7..32   meshlet id + 1                     (25 bits)
//   `visible_meshlets[i]` carries a raw meshlet id.
//
// (vs_vbuf_scene, fs_vbuf_scene) — scene-wide path (Phase 1.E):
//   Packing:
//     bit  0..7    triangle index inside the meshlet  (7 bits)
//     bit  7..32   visible-slot index + 1             (25 bits)
//   `visible_meshlets[i]` packs (instance_id<<16 | meshlet_idx); the
//   vertex shader decodes it to fetch transform from `instances[]` and
//   the fragment writes the *visible-slot index* so the deferred
//   shader can recover both instance + meshlet via one indirection.
//
// Encoded value 0 always means "background".

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
    // DAG: parent meshlet index (#442). u32::MAX sentinel = root.
    parent_meshlet_index: u32,
    aabb_max: vec3<f32>,
    // DAG: meshopt::simplify error this meshlet represents.
    lod_error: f32,
    bounds_center: vec3<f32>,
    bounding_radius: f32,
    cone_apex: vec3<f32>,
    cone_cutoff: f32,
    cone_axis: vec3<f32>,
    group_index: u32,
    children_group_index: u32,
    lod_level: u32,
    _pad4: u32,
    _pad5: u32,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<uniform> model: ModelUniforms;

@group(1) @binding(0) var<storage, read> vertices: array<MeshVertexStored>;
@group(1) @binding(1) var<storage, read> meshlet_vertices: array<u32>;
@group(1) @binding(2) var<storage, read> meshlet_triangles: array<u32>;
@group(1) @binding(3) var<storage, read> descriptors: array<MeshletDescriptor>;

@group(2) @binding(0) var<storage, read> visible_meshlets: array<u32>;

// Scene-path bind group — only bound for vs_vbuf_scene / fs_vbuf_scene.
struct MeshInstance {
    transform: mat4x4<f32>,
    mesh_id: u32,
    material_id: u32,
    lod_bias: f32,
    lod_force_level: i32,
}
@group(3) @binding(0) var<storage, read> instances: array<MeshInstance>;

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

@vertex
fn vs_vbuf_scene(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VsOut {
    let packed_visible = visible_meshlets[instance_index];
    let inst_id = packed_visible >> 16u;
    let meshlet_id = packed_visible & 0xffffu;

    let inst = instances[inst_id];
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
    let world_pos = inst.transform * vec4<f32>(pos, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    // Encode visible-slot index (+1 keeps 0 = background) so the
    // deferred shader recovers (instance_id, meshlet_idx) via one
    // indirection through visible_meshlets[].
    let visible_slot = instance_index + 1u;
    out.packed_id = (visible_slot << 7u) | (triangle_idx & 0x7Fu);
    return out;
}

@fragment
fn fs_vbuf_scene(input: VsOut) -> @location(0) u32 {
    return input.packed_id;
}

// shadow_depth.wgsl — the meshlet rasteriser, from a light, writing depth and nothing else (#476).

struct CascadeUniforms {
    // Light-space clip-from-world for the cascade being rendered.
    view_proj: mat4x4<f32>,
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
    parent_meshlet_index: u32,
    aabb_max: vec3<f32>,
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

// Stride 96 B, mirroring the cull side. A mismatch here reads a
// transform from the middle of the previous instance.
struct MeshInstance {
    transform: mat4x4<f32>,
    mesh_id: u32,
    material_id: u32,
    lod_bias: f32,
    lod_force_level: i32,
    group_base: u32,
    // #804 — per-instance bits; bit 0 is "receives shadows". Was
    // `_pad0`, so the 96-byte stride is unchanged.
    flags: u32,
    // Which layers the instance is in (#1218). `MeshInstance` in `scene.rs`.
    layers: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> cascade: CascadeUniforms;

@group(1) @binding(0) var<storage, read> vertices: array<MeshVertexStored>;
@group(1) @binding(1) var<storage, read> meshlet_vertices: array<u32>;
@group(1) @binding(2) var<storage, read> meshlet_triangles: array<u32>;
@group(1) @binding(3) var<storage, read> descriptors: array<MeshletDescriptor>;

@group(2) @binding(0) var<storage, read> visible_meshlets: array<u32>;

@group(3) @binding(0) var<storage, read> instances: array<MeshInstance>;

fn fetch_local_vertex_index(byte_offset: u32) -> u32 {
    let word_idx = byte_offset / 4u;
    let byte_in_word = byte_offset & 3u;
    let packed = meshlet_triangles[word_idx];
    return (packed >> (byte_in_word * 8u)) & 0xffu;
}

/// A caster's corner from the light: where it lands, and what a transparent one's coverage reads.
struct ShadowCorner {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) material: u32,
}

fn shadow_corner(vertex_index: u32, instance_index: u32) -> ShadowCorner {
    let packed_visible = visible_meshlets[instance_index];
    let inst_id = packed_visible >> 16u;
    let meshlet_id = packed_visible & 0xffffu;
    let desc = descriptors[meshlet_id];

    let triangle_idx = vertex_index / 3u;
    let corner_idx = vertex_index % 3u;

    var out: ShadowCorner;
    out.uv = vec2<f32>(0.0);
    out.material = 0u;
    // The draw is indirect with a fixed vertex count per meshlet, so the tail of a meshlet with
    // fewer triangles still runs. Sending those vertices outside the clip volume discards the
    // triangle without a branch anywhere else.
    if (triangle_idx >= desc.triangle_count) {
        out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }

    let byte_offset = desc.triangle_offset + triangle_idx * 3u + corner_idx;
    let local_vertex_idx = fetch_local_vertex_index(byte_offset);
    let global_vertex_idx = meshlet_vertices[desc.vertex_offset + local_vertex_idx];
    let v = vertices[global_vertex_idx];

    let pos = vec3<f32>(v.position[0], v.position[1], v.position[2]);
    let world_pos = instances[inst_id].transform * vec4<f32>(pos, 1.0);
    out.clip = cascade.view_proj * world_pos;
    out.uv = vec2<f32>(v.uv[0], v.uv[1]);
    out.material = instances[inst_id].material_id;
    return out;
}

@vertex
fn vs_shadow(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> @builtin(position) vec4<f32> {
    return shadow_corner(vertex_index, instance_index).clip;
}

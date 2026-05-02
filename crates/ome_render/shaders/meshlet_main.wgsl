// meshlet_main.wgsl — vertex-pull rasterizer for visible meshlets.
//
// Pairs with the compute culler (meshlet_cull.wgsl). One indirect
// draw call rasterizes every meshlet that survived culling:
//   instance_count = visible_count   (atomically appended by cull)
//   vertex_count   = MAX_TRIANGLES*3 (set on MeshletCull construction)
//
// Per-vertex routing (no vertex buffer; everything is a storage fetch):
//   instance_index → visible_meshlets[i]      → meshlet_id
//   vertex_index   = triangle_idx*3 + corner  → triangle/corner pair
//   triangle_idx ≥ desc.triangle_count        → clip-out (degenerate)
//
// Why pull-style: lets the rasterizer agree with the cull pass on
// meshlet identity by index alone — no vertex buffer means no
// per-meshlet `set_vertex_buffer` round-trip, which is what makes
// Nanite-class single-draw-call rendering possible.
//
// Output is the same world-normal-as-color debug shading the standard
// mesh pass uses. Materials / PBR arrive in PR-7 of #117.

struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

struct ModelUniforms {
    model: mat4x4<f32>,
}

// Use array<f32, N> (alignment 4) instead of vec3<f32> (alignment 16)
// so the storage layout matches the host-side `MeshVertex` exactly:
//   12 bytes position + 12 bytes normal + 8 bytes uv = 32 bytes.
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

// group(1) mirrors `meshlet_bind_group_layout` (gpu_meshlet.rs binding mod).
@group(1) @binding(0) var<storage, read> vertices: array<MeshVertexStored>;
@group(1) @binding(1) var<storage, read> meshlet_vertices: array<u32>;
// `meshlet_triangles` is a packed u8 buffer host-side. WGSL has no u8,
// so the rasterizer reads it as `array<u32>` (4 bytes per word) and
// extracts the relevant byte at vertex-fetch time.
@group(1) @binding(2) var<storage, read> meshlet_triangles: array<u32>;
@group(1) @binding(3) var<storage, read> descriptors: array<MeshletDescriptor>;

// group(2) carries the cull pass's surviving-meshlet list.
@group(2) @binding(0) var<storage, read> visible_meshlets: array<u32>;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
}

fn fetch_local_vertex_index(byte_offset: u32) -> u32 {
    let word_idx = byte_offset / 4u;
    let byte_in_word = byte_offset & 3u;
    let packed = meshlet_triangles[word_idx];
    return (packed >> (byte_in_word * 8u)) & 0xffu;
}

@vertex
fn vs_meshlet(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VsOut {
    let meshlet_id = visible_meshlets[instance_index];
    let desc = descriptors[meshlet_id];

    let triangle_idx = vertex_index / 3u;
    let corner_idx = vertex_index % 3u;

    var out: VsOut;
    if (triangle_idx >= desc.triangle_count) {
        // Push degenerate triangles outside the canonical clip volume
        // so the rasterizer culls them. `(2,2,2,1)` lies past every
        // [-w,w] face. Cheaper than a real branch in the FS path.
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.world_normal = vec3<f32>(0.0);
        return out;
    }

    let byte_offset = desc.triangle_offset + triangle_idx * 3u + corner_idx;
    let local_vertex_idx = fetch_local_vertex_index(byte_offset);

    let global_vertex_idx = meshlet_vertices[desc.vertex_offset + local_vertex_idx];
    let v = vertices[global_vertex_idx];

    let pos = vec3<f32>(v.position[0], v.position[1], v.position[2]);
    let nrm = vec3<f32>(v.normal[0], v.normal[1], v.normal[2]);

    let world_pos = model.model * vec4<f32>(pos, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    // Same scale assumption as mesh_main.wgsl — non-uniform scale gets
    // a transpose(inverse) in the eventual materials path (#130).
    out.world_normal = (model.model * vec4<f32>(nrm, 0.0)).xyz;
    return out;
}

@fragment
fn fs_meshlet(input: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(input.world_normal);
    return vec4<f32>(n * 0.5 + 0.5, 1.0);
}

// frustum_cull.wgsl — GPU-driven frustum cull over the multi-consumer
// BVH's `leaf_aabbs` (#115 PR-5 S5).
//
// One thread per leaf in original input order. Each thread:
//   1. Loads its leaf's AABB + flags from leaf_aabbs[gid.x].
//   2. Filters by IS_VISIBLE_MESH (bit 4) — non-mesh leaves write a
//      culled command and exit.
//   3. Tests AABB against the 6 frustum planes via positive-vertex
//      slab test. Plane convention: dot(plane.xyz, p) + plane.w >= 0
//      means inside; an AABB is fully outside when even its positive
//      vertex (the corner farthest in +normal) fails this.
//   4. Writes a DrawIndexedIndirectArgs[gid.x] entry with
//      instance_count = 1 (visible) or 0 (culled). The mesh pass
//      consumes this buffer via draw_indexed_indirect; entries with
//      instance_count == 0 are GPU-skipped (no draw work).
//
// GPU-driven: no readback per frame. The CPU only writes the frustum
// uniform once per camera change. Output is consumed by the mesh
// pass as INDIRECT — no compaction round-trip needed.

struct LeafAabb {
    aabb_min: vec3<f32>,
    flags: u32,
    aabb_max: vec3<f32>,
    entity_id: u32,
}

// Matches wgpu::util::DrawIndexedIndirectArgs (20 bytes, std430-clean).
// Fields in the order the GPU command processor expects them.
struct DrawIndexedIndirectArgs {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

// 112 bytes std140 uniform: 6 * 16 (planes) + 16 (n + index_count_per_mesh + 2x pad).
struct FrustumUniforms {
    // Plane equation per row: dot(planes[i].xyz, p) + planes[i].w >= 0
    // is the inside half-space. Order is irrelevant to correctness;
    // engine uses left/right/bottom/top/near/far by convention.
    planes: array<vec4<f32>, 6>,
    // Number of leaves to process. Threads with gid.x >= n early-out.
    n: u32,
    // index_count for the assumed-uniform mesh in S5 scope. Per-leaf
    // mesh metadata is a future extension when the engine ships
    // multiple distinct mesh instances behind the same BVH consumer.
    index_count_per_mesh: u32,
    _pad0: u32,
    _pad1: u32,
}

const IS_VISIBLE_MESH: u32 = 1u << 4u;

@group(0) @binding(0) var<storage, read> leaf_aabbs: array<LeafAabb>;
@group(0) @binding(1) var<storage, read_write> visible_indirect: array<DrawIndexedIndirectArgs>;
@group(0) @binding(2) var<uniform> frustum: FrustumUniforms;

// Plane-AABB positive-vertex test. Returns false iff the AABB is
// fully outside at least one of the 6 frustum planes (cull).
fn aabb_in_frustum(aabb_min: vec3<f32>, aabb_max: vec3<f32>) -> bool {
    for (var i: u32 = 0u; i < 6u; i = i + 1u) {
        let plane = frustum.planes[i];
        let n = plane.xyz;
        // The "positive vertex" is the AABB corner farthest in +n.
        // Per axis: pick max if n component is +, min if -.
        let pv = vec3<f32>(
            select(aabb_min.x, aabb_max.x, n.x >= 0.0),
            select(aabb_min.y, aabb_max.y, n.y >= 0.0),
            select(aabb_min.z, aabb_max.z, n.z >= 0.0),
        );
        if (dot(n, pv) + plane.w < 0.0) {
            return false;
        }
    }
    return true;
}

@compute @workgroup_size(64)
fn frustum_cull_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= frustum.n) {
        return;
    }
    let leaf = leaf_aabbs[i];

    // Default to culled. instance_count = 0 makes the GPU command
    // processor skip the draw entirely; the other fields stay valid
    // so a downstream debug tool can still read them.
    var args: DrawIndexedIndirectArgs;
    args.index_count = frustum.index_count_per_mesh;
    args.instance_count = 0u;
    args.first_index = 0u;
    args.base_vertex = 0;
    args.first_instance = i;

    // Two filters compose: the leaf must declare itself a visible
    // mesh AND its AABB must intersect the frustum.
    if ((leaf.flags & IS_VISIBLE_MESH) != 0u) {
        if (aabb_in_frustum(leaf.aabb_min, leaf.aabb_max)) {
            args.instance_count = 1u;
        }
    }

    visible_indirect[i] = args;
}

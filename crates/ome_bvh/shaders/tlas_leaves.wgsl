// tlas_leaves.wgsl — TLAS pass 2: write the N leaf nodes into the
// tail of the flat node array, encoded with the live-leaf payload.
//
// Layout (Karras-canonical, 2N-1 nodes):
//   tlas_nodes[0..N-1)     = internals (written by tlas_internal.wgsl)
//   tlas_nodes[N-1..2N-1)  = leaves (written here)
//
// Each leaf k:
//   - reads chunk_descriptors[live_chunk_indices[sorted_indices[k]]]
//     (slot-indexed lookup behind the Morton permutation).
//   - writes to tlas_nodes[(N-1) + k]
//   - left            = 0  (TLAS leaves have no leaves-payload offset;
//                           the BLAS pool is keyed off the chunk index
//                           encoded in `right_or_count` instead).
//   - right_or_count  = slot_idx | BVH_LEAF_FLAG  (encode_live with
//                       the actual `chunk_descriptors` slot index — the
//                       fragment shader does `chunk_descriptors[chunk_idx]`
//                       and must hit the live slot, not the contiguous
//                       Morton position which can change every rebuild).
//   - sets tlas_done[k] = 1, marking this leaf for the upcoming AABB
//     propagation pass (commit 7 will read this back).
//
// **TLAS_DEAD_FLAG (0x40000000u) is intentionally NOT set here.** A
// rebuild lays down LIVE leaves only; eviction marking is the job of
// the `remove_chunk` path which runs against a previously-built TLAS.

struct ChunkDescriptor {
    aabb_min: vec3<f32>,
    first_node: u32,
    aabb_max: vec3<f32>,
    node_count: u32,
    first_leaf: u32,
    leaf_count: u32,
    first_primitive: u32,
    primitive_count: u32,
    max_smoothness_radius: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

struct BvhNode {
    aabb_min: vec3<f32>,
    left: u32,
    aabb_max: vec3<f32>,
    right_or_count: u32,
}

struct TlasConfig {
    n: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<storage, read_write> tlas_nodes: array<BvhNode>;
@group(0) @binding(1) var<storage, read> sorted_indices: array<u32>;
@group(0) @binding(2) var<storage, read> chunk_descriptors: array<ChunkDescriptor>;
@group(0) @binding(3) var<storage, read_write> tlas_done: array<u32>;
@group(0) @binding(4) var<uniform> cfg: TlasConfig;
// `live_chunk_indices[k]` is the slot index of the k-th live chunk in
// `chunk_descriptors`. Resolved through `sorted_indices[k]` first so
// the Morton permutation lands on the right live slot.
@group(0) @binding(5) var<storage, read> live_chunk_indices: array<u32>;

const BVH_LEAF_FLAG: u32 = 0x80000000u;

@compute @workgroup_size(64)
fn tlas_leaves_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let k = gid.x;
    if k >= cfg.n {
        return;
    }
    // Edge case: N==1 → leaf_offset = 0, leaf at tlas_nodes[0] (root).
    let leaf_offset = select(cfg.n - 1u, 0u, cfg.n == 0u);
    let leaf_idx = leaf_offset + k;

    let live_pos = sorted_indices[k];
    let slot_idx = live_chunk_indices[live_pos];
    let desc = chunk_descriptors[slot_idx];

    tlas_nodes[leaf_idx].aabb_min = desc.aabb_min;
    tlas_nodes[leaf_idx].left = 0u;
    tlas_nodes[leaf_idx].aabb_max = desc.aabb_max;
    tlas_nodes[leaf_idx].right_or_count = slot_idx | BVH_LEAF_FLAG;

    tlas_done[k] = 1u;
}

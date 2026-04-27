// Karras LBVH — pass 1: write leaves into the flat node array.
//
// Layout (Karras-canonical, 2N-1 nodes):
//   nodes[0..N-1)     = internals (written by karras_internal pass)
//   nodes[N-1..2N-1)  = leaves (written here)
//
// Each leaf k:
//   - reads original_aabbs[sorted_indices[k]] (Morton-permuted lookup)
//   - writes to nodes[(N-1) + k]
//   - left field = k (index into the sorted leaves payload array)
//   - right_or_count = 1 | LEAF_FLAG (count = 1, leaf flag set)
//   - sets done[(N-1) + k] = 1, marking the leaf's AABB as finalized
//     for the bottom-up propagation pass.

struct GpuAabb {
    min: vec3<f32>,
    _pad0: f32,
    max: vec3<f32>,
    _pad1: f32,
}

struct BvhNode {
    aabb_min: vec3<f32>,
    left: u32,
    aabb_max: vec3<f32>,
    right_or_count: u32,
}

struct LbvhConfig {
    n: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<storage, read_write> nodes: array<BvhNode>;
@group(0) @binding(1) var<storage, read> original_aabbs: array<GpuAabb>;
@group(0) @binding(2) var<storage, read> sorted_indices: array<u32>;
@group(0) @binding(3) var<storage, read_write> done: array<u32>;
@group(0) @binding(4) var<uniform> cfg: LbvhConfig;

const LEAF_FLAG: u32 = 0x80000000u;

@compute @workgroup_size(64)
fn write_leaves_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let k = gid.x;
    if (k >= cfg.n) {
        return;
    }
    // Edge case: N==1 → leaf_offset = 0, leaf at nodes[0] (the root).
    let leaf_offset = select(cfg.n - 1u, 0u, cfg.n == 0u);
    let leaf_idx = leaf_offset + k;

    let original_idx = sorted_indices[k];
    let aabb = original_aabbs[original_idx];

    nodes[leaf_idx].aabb_min = aabb.min;
    nodes[leaf_idx].left = k;
    nodes[leaf_idx].aabb_max = aabb.max;
    nodes[leaf_idx].right_or_count = 1u | LEAF_FLAG;

    done[leaf_idx] = 1u;
}

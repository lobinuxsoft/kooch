// Karras LBVH — pass 3: bottom-up AABB propagation, one tree level
// per dispatch.
//
// WGSL has no cross-workgroup synchronisation primitive within a
// single dispatch (atomics provide ordering only on the atomic
// itself, not on adjacent memory). The dispatch boundary is the only
// portable cross-workgroup memory barrier, so we propagate one tree
// level per dispatch: in iteration k, every internal node whose two
// children were finalised in iterations 0..k-1 finalises itself.
//
// `done[node]`:
//   - leaves: set to 1 by pass 1 (write_leaves) — always ready.
//   - internals: set to 0 by pass 2 (karras_internal); set to 1
//                here once both children are done and the merge has
//                been written.
//
// The host loops this dispatch ⌈log₂ N⌉ + slack times, which
// guarantees every internal converges (Karras' index tie-break keeps
// the tree balanced even under duplicate Morton codes, so depth is
// always bounded by log₂ N).

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
@group(0) @binding(1) var<storage, read_write> done: array<u32>;
@group(0) @binding(2) var<uniform> cfg: LbvhConfig;

const VALUE_MASK: u32 = 0x7FFFFFFFu;

@compute @workgroup_size(64)
fn aabb_propagate_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i + 1u >= cfg.n) {
        return;
    }
    if (done[i] != 0u) {
        return;
    }

    let left_idx = nodes[i].left;
    let right_idx = nodes[i].right_or_count & VALUE_MASK;

    if (done[left_idx] == 0u || done[right_idx] == 0u) {
        return;
    }

    let lmin = nodes[left_idx].aabb_min;
    let lmax = nodes[left_idx].aabb_max;
    let rmin = nodes[right_idx].aabb_min;
    let rmax = nodes[right_idx].aabb_max;
    nodes[i].aabb_min = min(lmin, rmin);
    nodes[i].aabb_max = max(lmax, rmax);

    done[i] = 1u;
}

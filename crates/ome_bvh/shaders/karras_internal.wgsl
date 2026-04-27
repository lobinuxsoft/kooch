// Karras LBVH — pass 2: parallel construction of N-1 internal nodes.
//
// One thread per internal node i ∈ [0, N-1). The thread:
//   1. Determines the direction d ∈ {-1, +1} extending the range.
//   2. Computes the upper bound l_max via exponential search.
//   3. Refines to exact length l via binary search.
//   4. Computes the split position γ via binary search.
//   5. Resolves left/right child indices in the flat array (children
//      may be internals or leaves; leaves live at offset N-1).
//   6. Writes nodes[i] (AABB stays zero — propagated in pass 3).
//   7. Records parent[left] = parent[right] = i for the AABB pass.
//   8. Sets done[i] = 0, marking this internal's AABB as not yet
//      finalized for the bottom-up propagation pass.
//
// Reference: Karras 2012, "Maximizing Parallelism in the Construction
// of BVHs, Octrees, and k-d Trees" (Appendix A).

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
@group(0) @binding(1) var<storage, read> sorted_morton: array<u32>;
@group(0) @binding(2) var<storage, read_write> parents: array<u32>;
@group(0) @binding(3) var<storage, read_write> done: array<u32>;
@group(0) @binding(4) var<uniform> cfg: LbvhConfig;

// Longest common prefix of sorted_morton[i] and sorted_morton[j],
// with index tie-break to keep the algorithm well-defined when
// multiple leaves share the same Morton code. Returns -1 when j is
// out of range (signalling "no neighbour in this direction").
fn delta(i: u32, j: i32) -> i32 {
    if (j < 0 || u32(j) >= cfg.n) {
        return -1;
    }
    let ju = u32(j);
    let xi = sorted_morton[i];
    let xj = sorted_morton[ju];
    if (xi == xj) {
        // All 32 morton bits equal — fall through to index bits.
        return 32 + i32(countLeadingZeros(i ^ ju));
    }
    return i32(countLeadingZeros(xi ^ xj));
}

@compute @workgroup_size(64)
fn karras_internal_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i + 1u >= cfg.n) {
        return;
    }

    let i_s = i32(i);

    // 1. Direction.
    let d_plus = delta(i, i_s + 1);
    let d_minus = delta(i, i_s - 1);
    var d: i32 = -1;
    if (d_plus > d_minus) {
        d = 1;
    }

    // 2. Lower bound on common prefix.
    let delta_min = delta(i, i_s - d);

    // 3. Exponential search for the upper bound on length.
    var l_max: i32 = 2;
    loop {
        if (delta(i, i_s + l_max * d) <= delta_min) {
            break;
        }
        l_max = l_max * 2;
    }

    // 4. Binary search for the exact length.
    var l: i32 = 0;
    var t: i32 = l_max / 2;
    loop {
        if (t <= 0) { break; }
        if (delta(i, i_s + (l + t) * d) > delta_min) {
            l = l + t;
        }
        t = t / 2;
    }
    let j = i_s + l * d;

    // 5. Binary search for the split position.
    let delta_node = delta(i, j);
    var s: i32 = 0;
    var div: i32 = 2;
    loop {
        // Integer ceil-division for positive args: ceil(l/div) = (l + div - 1) / div.
        let t_split = (l + div - 1) / div;
        if (delta(i, i_s + (s + t_split) * d) > delta_node) {
            s = s + t_split;
        }
        if (t_split <= 1) { break; }
        div = div * 2;
    }
    // For d = -1 the split lies one index further left so γ is the
    // inclusive end of the left subrange.
    let gamma_s = i_s + s * d + min(d, 0);
    let gamma = u32(gamma_s);
    let first = u32(min(i_s, j));
    let last = u32(max(i_s, j));

    let leaf_offset = cfg.n - 1u;

    // 6. Resolve children. Single-leaf subranges land in the leaf
    // section [N-1, 2N-1); larger subranges become internal nodes
    // identified by their split position.
    var left_idx: u32;
    if (gamma == first) {
        left_idx = leaf_offset + gamma;
    } else {
        left_idx = gamma;
    }
    var right_idx: u32;
    if (gamma + 1u == last) {
        right_idx = leaf_offset + gamma + 1u;
    } else {
        right_idx = gamma + 1u;
    }

    // 7. Write the internal node. AABBs stay zero — pass 3 fills them.
    nodes[i].aabb_min = vec3<f32>(0.0);
    nodes[i].left = left_idx;
    nodes[i].aabb_max = vec3<f32>(0.0);
    nodes[i].right_or_count = right_idx;

    parents[left_idx] = i;
    parents[right_idx] = i;

    done[i] = 0u;
}

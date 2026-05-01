// tlas_internal.wgsl — TLAS pass 3: parallel construction of N-1
// internal nodes via Karras 2012's delta + range + split algorithm.
//
// Mirror of `karras_internal.wgsl` algorithmically (same delta, same
// exponential-then-binary search, same split resolution). The ONLY
// divergence is the indexing convention for `parents[]` and `done[]`:
//
//   tlas_done / tlas_parents indexing convention (TLAS-specific):
//     leaves    use index k ∈ [0, N)
//     internals use index N + i where i is the internal node idx ∈ [0, N-1)
//   This DIFFERS from the BLAS convention (leaves at [N-1, 2N-1) of done[]).
//   See accel/buffers.rs:64 for the rationale.
//
// `tlas_nodes` itself keeps the BLAS Karras layout (internals at
// `[0, N-1)`, leaves at `[N-1, 2N-1)`) because the AABB propagation
// pass and the legacy CPU traversal both depend on that node-index
// shape.
//
// One thread per internal node i ∈ [0, N-1). The thread:
//   1..5. Karras delta + range + split (identical to BLAS).
//   6.    Resolves left/right child node indices in `tlas_nodes`.
//   7.    Writes `tlas_nodes[i]` (AABB stays zero — propagated in pass 4).
//   8.    Records `parents[role_idx(child)] = i` for each child.
//   9.    Sets `done[N + i] = 0`, marking this internal as not yet
//         finalised for the bottom-up propagation pass.

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
@group(0) @binding(1) var<storage, read> sorted_morton: array<u32>;
@group(0) @binding(2) var<storage, read_write> parents: array<u32>;
@group(0) @binding(3) var<storage, read_write> done: array<u32>;
@group(0) @binding(4) var<uniform> cfg: TlasConfig;

// Translate a node index in `tlas_nodes` to its role-keyed position
// in `parents[]` / `done[]`. See module-level convention comment.
fn role_idx(node_idx: u32) -> u32 {
    let leaf_offset = cfg.n - 1u;
    if node_idx >= leaf_offset {
        return node_idx - leaf_offset;
    }
    return cfg.n + node_idx;
}

// Longest common prefix of sorted_morton[i] and sorted_morton[j],
// with index tie-break to keep the algorithm well-defined when
// multiple chunks share the same Morton code. Returns -1 when j is
// out of range. Byte-equivalent to the BLAS `delta` helper.
fn delta(i: u32, j: i32) -> i32 {
    if j < 0 || u32(j) >= cfg.n {
        return -1;
    }
    let ju = u32(j);
    let xi = sorted_morton[i];
    let xj = sorted_morton[ju];
    if xi == xj {
        return 32 + i32(countLeadingZeros(i ^ ju));
    }
    return i32(countLeadingZeros(xi ^ xj));
}

@compute @workgroup_size(64)
fn tlas_internal_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if i + 1u >= cfg.n {
        return;
    }

    let i_s = i32(i);

    // 1. Direction.
    let d_plus = delta(i, i_s + 1);
    let d_minus = delta(i, i_s - 1);
    var d: i32 = -1;
    if d_plus > d_minus {
        d = 1;
    }

    // 2. Lower bound on common prefix.
    let delta_min = delta(i, i_s - d);

    // 3. Exponential search for the upper bound on length.
    var l_max: i32 = 2;
    loop {
        if delta(i, i_s + l_max * d) <= delta_min {
            break;
        }
        l_max = l_max * 2;
    }

    // 4. Binary search for the exact length.
    var l: i32 = 0;
    var t: i32 = l_max / 2;
    loop {
        if t <= 0 { break; }
        if delta(i, i_s + (l + t) * d) > delta_min {
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
        let t_split = (l + div - 1) / div;
        if delta(i, i_s + (s + t_split) * d) > delta_node {
            s = s + t_split;
        }
        if t_split <= 1 { break; }
        div = div * 2;
    }
    let gamma_s = i_s + s * d + min(d, 0);
    let gamma = u32(gamma_s);
    let first = u32(min(i_s, j));
    let last = u32(max(i_s, j));

    let leaf_offset = cfg.n - 1u;

    // 6. Resolve children. Single-leaf subranges land in the leaf
    // section [N-1, 2N-1); larger subranges become internal nodes
    // identified by their split position.
    var left_idx: u32;
    if gamma == first {
        left_idx = leaf_offset + gamma;
    } else {
        left_idx = gamma;
    }
    var right_idx: u32;
    if gamma + 1u == last {
        right_idx = leaf_offset + gamma + 1u;
    } else {
        right_idx = gamma + 1u;
    }

    // 7. Write the internal node. AABBs stay zero — pass 4 fills them.
    tlas_nodes[i].aabb_min = vec3<f32>(0.0);
    tlas_nodes[i].left = left_idx;
    tlas_nodes[i].aabb_max = vec3<f32>(0.0);
    tlas_nodes[i].right_or_count = right_idx;

    // 8. Parent pointers in TLAS convention (role_idx).
    parents[role_idx(left_idx)] = i;
    parents[role_idx(right_idx)] = i;

    // 9. Mark this internal's AABB as not yet finalised.
    done[cfg.n + i] = 0u;
}

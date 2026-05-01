// tlas_aabb.wgsl — TLAS pass 4: bottom-up AABB propagation, one tree
// level per dispatch.
//
// Mirror of `karras_aabb.wgsl` algorithmically (same multi-dispatch
// convergence loop driven by the host; same per-node done flag check).
// The ONLY divergence is the indexing convention for `done[]`:
//
//   tlas_done indexing convention (TLAS-specific):
//     leaves    use index k ∈ [0, N)           — set by tlas_leaves
//     internals use index N + i where i ∈ [0, N-1) — set by tlas_internal/here
//   This DIFFERS from the BLAS convention (leaves at [N-1, 2N-1) of done[]).
//   See accel/buffers.rs:64 for the rationale.
//
// `tlas_nodes` itself keeps the BLAS Karras layout (internals at
// `[0, N - 1)`, leaves at `[N - 1, 2N - 1)`), so `nodes[i].left` /
// `nodes[i].right_or_count & VALUE_MASK` are valid node indices and
// the AABB lookup is direct.
//
// WGSL has no cross-workgroup synchronisation primitive within a
// single dispatch (atomics give ordering only on the atomic itself,
// not on adjacent memory). The dispatch boundary is the only
// portable cross-workgroup memory barrier — the host loops this
// dispatch `aabb_iterations(n) = 2 * log2(n) + 4` times so every
// internal converges, even under duplicate Morton codes (Karras'
// index tie-break keeps tree depth bounded by log₂ N).
//
// `parents[]` is bound for symmetry with the BLAS (the bottom-up
// walk variant could use it) but unused in this multi-dispatch
// convergence variant — kept in the bind-group layout so the
// pipeline can be swapped to a parents-walk implementation later
// without re-plumbing the dispatch.

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
@group(0) @binding(1) var<storage, read> parents: array<u32>;
@group(0) @binding(2) var<storage, read_write> done: array<u32>;
@group(0) @binding(3) var<uniform> cfg: TlasConfig;

const VALUE_MASK: u32 = 0x7FFFFFFFu;

// Translate a `tlas_nodes` index to its role-keyed position in
// `done[]` / `parents[]` (TLAS convention). Mirrors the helper in
// `tlas_internal.wgsl`.
fn role_idx(node_idx: u32) -> u32 {
    let leaf_offset = cfg.n - 1u;
    if node_idx >= leaf_offset {
        return node_idx - leaf_offset;
    }
    return cfg.n + node_idx;
}

@compute @workgroup_size(64)
fn tlas_aabb_propagate_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if i + 1u >= cfg.n {
        return;
    }

    let my_role = cfg.n + i;
    if done[my_role] != 0u {
        return;
    }

    let left_idx = tlas_nodes[i].left;
    let right_idx = tlas_nodes[i].right_or_count & VALUE_MASK;

    let left_role = role_idx(left_idx);
    let right_role = role_idx(right_idx);

    if done[left_role] == 0u || done[right_role] == 0u {
        return;
    }

    let lmin = tlas_nodes[left_idx].aabb_min;
    let lmax = tlas_nodes[left_idx].aabb_max;
    let rmin = tlas_nodes[right_idx].aabb_min;
    let rmax = tlas_nodes[right_idx].aabb_max;
    tlas_nodes[i].aabb_min = min(lmin, rmin);
    tlas_nodes[i].aabb_max = max(lmax, rmax);

    done[my_role] = 1u;
}

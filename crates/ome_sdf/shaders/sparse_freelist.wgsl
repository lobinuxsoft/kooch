// Reusable atomic free-list helpers for the sparse SDF subgrid pool
// (issue #136). Consumer shaders concat this fragment via the
// `SPARSE_FREELIST_WGSL` Rust constant.
//
// Bindings (group 0 reserved for sparse-storage primitives — consumer
// shaders may add further bindings to group 0 or shift their own to
// group 1+):
//
//   @group(0) @binding(0) var<storage, read_write> sparse_free_list:
//   @group(0) @binding(1) var<storage, read_write> sparse_counters:
//
// `SparseCounters` matches the 16 B host layout in
// `ome_sdf::sparse::FREELIST_COUNTERS_SIZE`.

struct SparseCounters {
    // Top of the free stack. `free_list[free_top - 1]` is the next
    // index a `sparse_pop_subgrid_index` call returns. Initialised by
    // `SparseGrid::new` to `max_subgrids`.
    free_top: atomic<u32>,
    // Number of `sparse_pop_subgrid_index` calls that hit an empty
    // free list and returned `SPARSE_ALLOC_FAILED`. Diagnostic only;
    // never read by the lookup path.
    alloc_failed_count: atomic<u32>,
    // Cumulative successful pop count. Incremented every time
    // `sparse_pop_subgrid_index` returns a real index (post-CAS
    // success). Persists across cascade runs — host clears via
    // `queue.write_buffer` if a fresh window is needed. Read by the
    // metrics pass (S8) to surface allocation churn.
    alloc_count_total: atomic<u32>,
    // Cumulative push count. Incremented in `sparse_push_subgrid_index`.
    // Same persistence + read story as `alloc_count_total`.
    free_count_total: atomic<u32>,
}

@group(0) @binding(0) var<storage, read_write> sparse_free_list: array<u32>;
@group(0) @binding(1) var<storage, read_write> sparse_counters: SparseCounters;

// Sentinel returned by `sparse_pop_subgrid_index` when the free list
// is empty. Mirrors the host-side `ALLOC_FAILED_SENTINEL`.
const SPARSE_ALLOC_FAILED: u32 = 0xFFFFFFFEu;

// Pop a subgrid index off the free stack.
//
// Returns the popped index on success, or `SPARSE_ALLOC_FAILED` when
// the pool is exhausted (and increments `sparse_counters.alloc_failed_count`).
//
// Compare-exchange loop guards against the underflow that a naive
// `atomicSub` would hit when two invocations race on a free_top of 1.
//
// Mixing pops and pushes in the same compute pass is undefined —
// callers must keep allocation passes (consumers) and free passes
// (producers) on separate dispatches with the implicit storage
// barrier between them.
fn sparse_pop_subgrid_index() -> u32 {
    // Single exit point — early `return` from inside `loop` confuses
    // naga's reachability analysis on the implicit fall-through past
    // the loop block. The boolean trip-flag keeps validation happy
    // while preserving the compare-exchange semantics.
    var out: u32 = SPARSE_ALLOC_FAILED;
    var done: bool = false;
    loop {
        if (done) { break; }
        let cur = atomicLoad(&sparse_counters.free_top);
        if (cur == 0u) {
            atomicAdd(&sparse_counters.alloc_failed_count, 1u);
            done = true;
            continue;
        }
        let result = atomicCompareExchangeWeak(
            &sparse_counters.free_top,
            cur,
            cur - 1u,
        );
        if (result.exchanged) {
            out = sparse_free_list[cur - 1u];
            atomicAdd(&sparse_counters.alloc_count_total, 1u);
            done = true;
        }
        // Lost the race; another invocation popped first. Retry.
    }
    return out;
}

// Push a freed subgrid index back onto the stack. Caller invariants:
// `idx` was previously returned by `sparse_pop_subgrid_index` (so it
// is in `0..max_subgrids`) and has not been pushed in the same pass
// (no double-free).
fn sparse_push_subgrid_index(idx: u32) {
    let slot = atomicAdd(&sparse_counters.free_top, 1u);
    sparse_free_list[slot] = idx;
    atomicAdd(&sparse_counters.free_count_total, 1u);
}

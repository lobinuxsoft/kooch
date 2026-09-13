// Atomic free-list helpers for the subgrid pool (#136), concatenated via `SPARSE_FREELIST_WGSL`.
// Group 0 bindings 0–1; `SparseCounters` matches the 16 B `FREELIST_COUNTERS_SIZE` on the host.

struct SparseCounters {
    // Top of the free stack. `free_list[free_top - 1]` is the next
    // index a `sparse_pop_subgrid_index` call returns. Initialised by
    // `SparseGrid::new` to `max_subgrids`.
    free_top: atomic<u32>,
    // Number of `sparse_pop_subgrid_index` calls that hit an empty
    // free list and returned `SPARSE_ALLOC_FAILED`. Diagnostic only;
    // never read by the lookup path.
    alloc_failed_count: atomic<u32>,
    // Cumulative successful pops, persisting across cascades until the host clears it — the metrics
    // pass reads allocation churn from here.
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

// Pops an index, or `SPARSE_ALLOC_FAILED` when exhausted. Compare-exchange, since `atomicSub`
// underflows when two invocations race on a top of 1.
// Pops and pushes must live in separate dispatches.
fn sparse_pop_subgrid_index() -> u32 {
    // Single exit point: an early `return` inside `loop` confuses naga's reachability analysis.
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

// Pushes a freed index back. The caller guarantees it came from a pop and is not pushed twice in
// one pass.
fn sparse_push_subgrid_index(idx: u32) {
    let slot = atomicAdd(&sparse_counters.free_top, 1u);
    sparse_free_list[slot] = idx;
    atomicAdd(&sparse_counters.free_count_total, 1u);
}

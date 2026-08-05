// Sparse SDF metrics pass — telemetry sink at the tail of the
// cascade. One workgroup, one thread; reads each LOD's freelist
// counters and writes the aggregated `SparseMetrics` struct.
//
// # Bindings
//
// - `@group(0) @binding(0..3)` — per-LOD counters (read). Same layout
//   as `SparseCounters` in `sparse_freelist.wgsl`.
// - `@group(0) @binding(4)` — output metrics buffer (read_write).
//
// # Output layout
//
// Mirrors the host `Metrics` struct (`metrics.rs`):
// ```
// active_subgrids[LOD_COUNT]  // u32 each, max_subgrids - free_top
// alloc_count_total           // sum of per-LOD alloc counters
// free_count_total            // sum of per-LOD free counters
// ```
// Total = (LOD_COUNT + 2) × 4 = 24 B for LOD_COUNT = 4.
//
// # Why an active count derived from `free_top`
//
// The freelist is the single source of truth for "subgrids handed out
// to this LOD's atlas". Sampling `root_indices[*]` would be more
// faithful (it tracks cells with valid pointers, not just allocations)
// but requires a 4096-entry reduction per LOD. The freelist scalar
// nails the same number for the canonical cascade where every pop
// publishes its idx into root_indices, and a divergence (cells with
// `root_indices == ALLOC_FAILED`) is already surfaced via
// `alloc_failed_count` for the host to log separately.

struct MetricsCounters {
    free_top: u32,
    alloc_failed_count: u32,
    alloc_count_total: u32,
    free_count_total: u32,
}

struct SparseMetrics {
    active_lod0: u32,
    active_lod1: u32,
    active_lod2: u32,
    active_lod3: u32,
    alloc_count_total: u32,
    free_count_total: u32,
}

override METRICS_MAX_SUBGRIDS: u32 = 1024u;

@group(0) @binding(0) var<storage, read> metrics_counters_lod0: MetricsCounters;
@group(0) @binding(1) var<storage, read> metrics_counters_lod1: MetricsCounters;
@group(0) @binding(2) var<storage, read> metrics_counters_lod2: MetricsCounters;
@group(0) @binding(3) var<storage, read> metrics_counters_lod3: MetricsCounters;
@group(0) @binding(4) var<storage, read_write> metrics_out: SparseMetrics;

@compute @workgroup_size(1)
fn metrics_main() {
    let active0 = METRICS_MAX_SUBGRIDS - metrics_counters_lod0.free_top;
    let active1 = METRICS_MAX_SUBGRIDS - metrics_counters_lod1.free_top;
    let active2 = METRICS_MAX_SUBGRIDS - metrics_counters_lod2.free_top;
    let active3 = METRICS_MAX_SUBGRIDS - metrics_counters_lod3.free_top;

    let alloc_total =
        metrics_counters_lod0.alloc_count_total
        + metrics_counters_lod1.alloc_count_total
        + metrics_counters_lod2.alloc_count_total
        + metrics_counters_lod3.alloc_count_total;
    let free_total =
        metrics_counters_lod0.free_count_total
        + metrics_counters_lod1.free_count_total
        + metrics_counters_lod2.free_count_total
        + metrics_counters_lod3.free_count_total;

    metrics_out.active_lod0 = active0;
    metrics_out.active_lod1 = active1;
    metrics_out.active_lod2 = active2;
    metrics_out.active_lod3 = active3;
    metrics_out.alloc_count_total = alloc_total;
    metrics_out.free_count_total = free_total;
}

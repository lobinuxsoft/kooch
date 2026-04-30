// onesweep_init.wgsl — clear scratch buffers before a sort run.
//
// Two responsibilities:
//
// 1. Zero the global histogram (4 passes × 256 buckets × u32 = 4 KB).
// 2. Zero the partition status descriptors for one pass — used by the
//    decoupled-lookback chained scan in `onesweep_scatter.wgsl`. The
//    Rust dispatcher binds a fresh descriptor buffer per pass (or a
//    cleared section of one larger buffer) so each pass starts with
//    every descriptor in the INVALID state.
//
// Single workgroup per dispatch is enough — the buffers are small and
// the init runs once per build. We keep the kernel here rather than
// inlining a `queue.write_buffer` zero so timestamp queries in the
// `BvhGpuBuilder` see the init time as a compute pass like the others.

@group(0) @binding(0) var<storage, read_write> global_histogram: array<u32>;
@group(0) @binding(1) var<storage, read_write> partition_descriptors: array<atomic<u32>>;

struct InitConfig {
    histogram_count: u32,        // 4 * 256 = 1024
    descriptor_count: u32,       // partition_count * 256 (per pass)
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(2) var<uniform> cfg: InitConfig;

@compute @workgroup_size(256)
fn init_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;

    // Histogram. Strided loop — `histogram_count` is `4 * 256 = 1024`,
    // bigger than the 256-thread workgroup, so a flat `if i < count`
    // would only clear entries 0..255 and leave 256..1023 carrying
    // residue from the previous build. The next build's
    // `onesweep_global_histogram.wgsl` adds via `atomicAdd`, so any
    // residue corrupts every pass after the first byte (#333). The
    // bug was masked when consecutive builds saw cached zeroes; a
    // `device.poll` between submissions flushed the cache and
    // surfaced the partial clear as 2-cycle Karras topologies.
    var h_idx = i;
    while h_idx < cfg.histogram_count {
        global_histogram[h_idx] = 0u;
        h_idx = h_idx + 256u;
    }

    // Then descriptors. May be many more than threads — strided loop
    // covers the rest. Atomic store with relaxed ordering is enough
    // because no other workgroup is reading these yet (scatter pass
    // dispatches strictly after init via a separate submission).
    var idx = i;
    while idx < cfg.descriptor_count {
        atomicStore(&partition_descriptors[idx], 0u);  // FLAG_INVALID = 0, value = 0
        idx = idx + 256u;
    }
}

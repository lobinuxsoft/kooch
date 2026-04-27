// onesweep_exclusive_scan.wgsl — exclusive prefix sum of the 256-bucket
// histogram for one radix pass. The result is the per-bucket starting
// offset in the sorted output, used by the scatter pass.
//
// One workgroup per pass (4 dispatches total); 256 threads, one per
// bucket. In-place exclusive scan via Hillis-Steele (log2(N) =
// 8 phases) using workgroup-scan_buf memory. Hillis-Steele gives the
// simplest correctness proof for N=256 — a Brent-Kung work-efficient
// scan would halve the memory traffic but the bucket count is small
// enough that it doesn't matter at this scale.
//
// Output `global_histogram[pass * 256 + bucket]` is overwritten with
// the exclusive prefix sum: count of keys whose digit at this pass
// is strictly less than `bucket`. Bucket 0 → 0; bucket 255 → total
// count of keys minus the count of bucket 255.

const RADIX_BUCKETS: u32 = 256u;

struct ScanConfig {
    pass_index: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<storage, read_write> global_histogram: array<u32>;
@group(0) @binding(1) var<uniform> cfg: ScanConfig;

var<workgroup> scan_buf: array<u32, 256>;

@compute @workgroup_size(256)
fn scan_main(@builtin(local_invocation_id) lid: vec3<u32>) {
    let tid = lid.x;
    let base = cfg.pass_index * RADIX_BUCKETS;

    // Load this pass's slice of the global histogram into scan_buf memory.
    scan_buf[tid] = global_histogram[base + tid];
    workgroupBarrier();

    // Hillis-Steele inclusive scan, log2(256) = 8 phases.
    var offset = 1u;
    for (var d = 0u; d < 8u; d = d + 1u) {
        var v: u32 = 0u;
        if tid >= offset {
            v = scan_buf[tid - offset];
        }
        workgroupBarrier();
        if tid >= offset {
            scan_buf[tid] = scan_buf[tid] + v;
        }
        workgroupBarrier();
        offset = offset * 2u;
    }

    // Convert inclusive → exclusive: shift right by one (slot 0 → 0,
    // slot k → inclusive[k-1]).
    var exclusive: u32 = 0u;
    if tid > 0u {
        exclusive = scan_buf[tid - 1u];
    }
    workgroupBarrier();

    // Write exclusive prefix back to the histogram slice.
    global_histogram[base + tid] = exclusive;
}

// onesweep_global_histogram.wgsl — count digits across all 4 passes in
// a single dispatch.
//
// One workgroup per `ITEMS_PER_TILE = 3072` input keys; each thread
// reads `ITEMS_PER_THREAD = 12` keys with stride `WORKGROUP_SIZE = 256`
// (coalesced access). For each key, the 4 byte-digits are extracted
// and accumulated into a workgroup-local histogram of
// `RADIX_PASSES * RADIX_BUCKETS = 4 * 256 = 1024` u32 slots. After the
// barrier, each thread flushes its 4 owned slots (one per pass at the
// thread's local id) to the global histogram via `atomicAdd`.
//
// Output `global_histogram[pass * 256 + bucket]` is the inclusive
// total count of keys with `(key >> (pass * 8)) & 0xFF == bucket`.
// The exclusive prefix sum (used by the scatter as the per-bucket
// starting offset) is computed by the next shader,
// `onesweep_exclusive_scan.wgsl`.

const WORKGROUP_SIZE: u32 = 256u;
const RADIX_BUCKETS: u32 = 256u;
const RADIX_PASSES: u32 = 4u;
const ITEMS_PER_THREAD: u32 = 12u;
const ITEMS_PER_TILE: u32 = WORKGROUP_SIZE * ITEMS_PER_THREAD; // 3072

struct HistogramConfig {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<storage, read> keys: array<u32>;
@group(0) @binding(1) var<storage, read_write> global_histogram: array<atomic<u32>>;
@group(0) @binding(2) var<uniform> cfg: HistogramConfig;

// 4 passes × 256 buckets = 1024 atomic u32 in workgroup memory.
// Atomic because multiple threads in the same workgroup may target
// the same digit slot in the same key-loop iteration.
var<workgroup> local_histograms: array<atomic<u32>, 1024>;

@compute @workgroup_size(256)
fn count_main(
    @builtin(local_invocation_id) lid: vec3<u32>,
    @builtin(workgroup_id) wgid: vec3<u32>,
) {
    let tid = lid.x;

    // Clear local histograms — each thread owns one slot per pass.
    for (var p = 0u; p < RADIX_PASSES; p = p + 1u) {
        atomicStore(&local_histograms[p * RADIX_BUCKETS + tid], 0u);
    }
    workgroupBarrier();

    // Count digits in this workgroup's tile. Stride pattern:
    // thread `tid` reads keys at positions `tile_start + k * WG + tid`
    // for k in [0, 12). Coalesced — consecutive threads access
    // consecutive memory.
    let tile_start = wgid.x * ITEMS_PER_TILE;
    for (var k = 0u; k < ITEMS_PER_THREAD; k = k + 1u) {
        let idx = tile_start + k * WORKGROUP_SIZE + tid;
        if idx >= cfg.count {
            break;
        }
        let key = keys[idx];
        for (var p = 0u; p < RADIX_PASSES; p = p + 1u) {
            let digit = (key >> (p * 8u)) & 0xFFu;
            atomicAdd(&local_histograms[p * RADIX_BUCKETS + digit], 1u);
        }
    }
    workgroupBarrier();

    // Flush local histograms to global. Each thread owns one bucket
    // per pass (its local invocation id).
    for (var p = 0u; p < RADIX_PASSES; p = p + 1u) {
        let local = atomicLoad(&local_histograms[p * RADIX_BUCKETS + tid]);
        if local > 0u {
            atomicAdd(&global_histogram[p * RADIX_BUCKETS + tid], local);
        }
    }
}

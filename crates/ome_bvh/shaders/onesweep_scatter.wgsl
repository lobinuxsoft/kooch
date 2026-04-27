// onesweep_scatter.wgsl — per-pass scatter with decoupled-lookback
// chained scan.
//
// One workgroup per partition (`ITEMS_PER_TILE = 3072` keys / partition);
// 256 threads per workgroup, 12 keys per thread.
//
// Algorithm (Adinets/Merrill 2022):
//
// 1. Load this tile's keys + values into workgroup memory.
// 2. Each thread atomic-adds its 12 digits (current pass's byte) into
//    a workgroup-local 256-bucket atomic histogram.
// 3. Hillis-Steele exclusive scan over the histogram → each bucket
//    holds its starting offset within the tile.
// 4. Decoupled lookback chained scan:
//    - Partition 0 publishes PREFIX directly (== local aggregate).
//    - Partition N>0 publishes AGGREGATE, walks back through
//      partitions 0..N-1 via spin-load on `atomicLoad`, sums their
//      AGGREGATEs until a PREFIX is found, publishes its own PREFIX.
//    - `lookback[bucket]` ends up as the cumulative count of items
//      with that digit in partitions 0..partition-1.
// 5. Scatter: per-thread atomic-add a workgroup-shared per-bucket
//    cursor to compute the local rank, then write to:
//       global_pos = global_histogram[bucket] + lookback[bucket] + local_rank
//
// `storageBarrier` after every atomic-store on descriptors so lookback
// walks see fresh data. Lookback spin-loop on atomicLoad handles the
// eventual-consistency.
//
// Status descriptor packing: high 2 bits flag, low 30 bits value.
// FLAG_INVALID=0, FLAG_AGGREGATE=1, FLAG_PREFIX=2.

const WORKGROUP_SIZE: u32 = 256u;
const RADIX_BUCKETS: u32 = 256u;
const ITEMS_PER_THREAD: u32 = 12u;
const ITEMS_PER_TILE: u32 = WORKGROUP_SIZE * ITEMS_PER_THREAD; // 3072

const FLAG_INVALID: u32 = 0u;
const FLAG_AGGREGATE: u32 = 1u;
const FLAG_PREFIX: u32 = 2u;
const FLAG_SHIFT: u32 = 30u;
const VALUE_MASK: u32 = 0x3FFFFFFFu;

struct ScatterConfig {
    count: u32,
    pass_shift: u32,
    partition_count: u32,
    _pad: u32,
}

@group(0) @binding(0) var<storage, read> keys_in: array<u32>;
@group(0) @binding(1) var<storage, read> values_in: array<u32>;
@group(0) @binding(2) var<storage, read_write> keys_out: array<u32>;
@group(0) @binding(3) var<storage, read_write> values_out: array<u32>;
@group(0) @binding(4) var<storage, read> global_histogram: array<u32>;
@group(0) @binding(5) var<storage, read_write> partition_descriptors: array<atomic<u32>>;
@group(0) @binding(6) var<uniform> cfg: ScatterConfig;

// Workgroup-local atomic histogram. Each bucket counts atomic-add
// across all 12 keys × 256 threads = 3072 increments (same key may
// land in same bucket from many threads simultaneously).
var<workgroup> local_hist: array<atomic<u32>, 256>;

// Tile cache. 3072 u32 keys + 3072 u32 values = 24 KB workgroup
// memory. RDNA 4 / RDNA 2 expose ≥ 32 KB by default; the elevated
// limits configured in #258 ensure headroom.
var<workgroup> tile_keys: array<u32, 3072>;
var<workgroup> tile_values: array<u32, 3072>;

// Hillis-Steele scratch (after the scan, holds the inclusive prefix).
var<workgroup> scan_buf: array<u32, 256>;

// Per-bucket cumulative count from prior partitions, filled by the
// lookback walk.
var<workgroup> lookback: array<u32, 256>;

// Per-bucket scatter cursor — atomic so each thread gets a unique
// rank within its bucket as it scatters its 12 keys.
var<workgroup> bucket_cursor: array<atomic<u32>, 256>;

fn extract_digit(key: u32) -> u32 {
    return (key >> cfg.pass_shift) & 0xFFu;
}

@compute @workgroup_size(256)
fn scatter_main(
    @builtin(local_invocation_id) lid: vec3<u32>,
    @builtin(workgroup_id) wgid: vec3<u32>,
) {
    let tid = lid.x;
    let part_id = wgid.x;
    let tile_start = part_id * ITEMS_PER_TILE;

    // -- 1. Load tile + clear local hist -------------------------------
    for (var k = 0u; k < ITEMS_PER_THREAD; k = k + 1u) {
        let idx = tile_start + k * WORKGROUP_SIZE + tid;
        let local_pos = k * WORKGROUP_SIZE + tid;
        if idx < cfg.count {
            tile_keys[local_pos] = keys_in[idx];
            tile_values[local_pos] = values_in[idx];
        }
    }
    atomicStore(&local_hist[tid], 0u);
    atomicStore(&bucket_cursor[tid], 0u);
    workgroupBarrier();

    // -- 2. Count digits via workgroup atomics -----------------------
    for (var k = 0u; k < ITEMS_PER_THREAD; k = k + 1u) {
        let idx = tile_start + k * WORKGROUP_SIZE + tid;
        if idx < cfg.count {
            let local_pos = k * WORKGROUP_SIZE + tid;
            let digit = extract_digit(tile_keys[local_pos]);
            atomicAdd(&local_hist[digit], 1u);
        }
    }
    workgroupBarrier();

    // -- 3. Hillis-Steele inclusive scan -----------------------------
    let local_aggregate = atomicLoad(&local_hist[tid]);
    scan_buf[tid] = local_aggregate;
    workgroupBarrier();

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
    // scan_buf[tid] now holds the inclusive prefix.

    // -- 4. Publish AGGREGATE / PREFIX + lookback walk ---------------
    let desc_idx = part_id * RADIX_BUCKETS + tid;

    if part_id == 0u {
        // No predecessors — local aggregate IS the prefix.
        atomicStore(
            &partition_descriptors[desc_idx],
            (FLAG_PREFIX << FLAG_SHIFT) | (local_aggregate & VALUE_MASK),
        );
        lookback[tid] = 0u;
    } else {
        // Publish AGGREGATE so successors can find us during their
        // own lookback.
        atomicStore(
            &partition_descriptors[desc_idx],
            (FLAG_AGGREGATE << FLAG_SHIFT) | (local_aggregate & VALUE_MASK),
        );
        storageBarrier();

        // Walk back; spin-load on each predecessor until its flag
        // transitions out of INVALID.
        var sum: u32 = 0u;
        var p_signed: i32 = i32(part_id) - 1;
        loop {
            if p_signed < 0 { break; }
            let prev_idx = u32(p_signed) * RADIX_BUCKETS + tid;
            var prev: u32 = 0u;
            loop {
                prev = atomicLoad(&partition_descriptors[prev_idx]);
                if (prev >> FLAG_SHIFT) != FLAG_INVALID {
                    break;
                }
            }
            let flag = prev >> FLAG_SHIFT;
            sum = sum + (prev & VALUE_MASK);
            if flag == FLAG_PREFIX {
                break;
            }
            p_signed = p_signed - 1;
        }

        // Publish own PREFIX.
        let prefix = sum + local_aggregate;
        atomicStore(
            &partition_descriptors[desc_idx],
            (FLAG_PREFIX << FLAG_SHIFT) | (prefix & VALUE_MASK),
        );
        lookback[tid] = sum;
    }
    workgroupBarrier();

    // -- 5. Scatter --------------------------------------------------
    // Stability matters for LSD radix sort: each pass must preserve
    // the relative order of items with equal digits, otherwise lower-
    // byte orderings established by earlier passes are lost.
    //
    // Serialising the cursor increments by `local_pos` order achieves
    // stability: items with the same bucket get consecutive cursor
    // values in `local_pos` order. The owner thread of each
    // `local_pos` does the work; all other threads idle through that
    // iteration and join the barrier.
    //
    // Cost: O(ITEMS_PER_TILE) sequential iterations per workgroup
    // = 3072 barriers. Slow vs a fully-parallel rank, but correct.
    // Optimisation tracked as follow-up: replace with a
    // subgroup-parallel rank using `subgroupExclusiveAdd` over a
    // per-key bucket-match mask, ~30× speedup on RDNA at the cost of
    // requiring `Features::SUBGROUP` (which both target adapters
    // expose). Filed alongside.
    // `global_histogram` is a flat 4*256-entry buffer; this pass's
    // prefix sums live at `pass_index * 256 + digit`.
    let pass_hist_base = (cfg.pass_shift / 8u) * RADIX_BUCKETS;

    for (var lp = 0u; lp < ITEMS_PER_TILE; lp = lp + 1u) {
        let owner_tid = lp % WORKGROUP_SIZE;
        let idx = tile_start + lp;
        if tid == owner_tid && idx < cfg.count {
            let key = tile_keys[lp];
            let val = tile_values[lp];
            let digit = extract_digit(key);
            let cursor = atomicAdd(&bucket_cursor[digit], 1u);
            let global_offset =
                global_histogram[pass_hist_base + digit] + lookback[digit] + cursor;
            keys_out[global_offset] = key;
            values_out[global_offset] = val;
        }
        workgroupBarrier();
    }
}

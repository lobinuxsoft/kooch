//! GPU-side data layouts for the onesweep radix sort.
//!
//! Onesweep (Adinets/Merrill 2022, "Decoupled Lookback Chained Scan
//! with Decoupled Lookback") is the state-of-art GPU radix sort
//! shipped here for the LBVH build pipeline. 4 passes × 8-bit radix
//! over 32-bit Morton keys; each pass is a single global kernel with
//! intra-workgroup count + workgroup-level prefix-sum + decoupled
//! lookback chained scan + scatter, all in one dispatch.
//!
//! References:
//! - Original paper: <https://arxiv.org/abs/2206.01784>
//! - NVIDIA reference: <https://github.com/b0nes164/GPUSorting>
//! - WGSL adaptation patterns (4-bit LSD, no subgroup): <https://github.com/kishimisu/WebGPU-Radix-Sort>
//!
//! This module provides the host-side data layouts; the shaders live
//! in `crates/ome_bvh/shaders/onesweep_*.wgsl` and the dispatch in
//! `gpu/sort.rs`.

use bytemuck::{Pod, Zeroable};

/// Number of bits per radix digit. 8 → 256 buckets per pass, 4 passes
/// for a 32-bit key. Onesweep paper recommends 8 for the CUDA reference;
/// we keep 8 for the WGSL port — it's the sweet spot for histogram
/// register pressure vs total pass count.
pub const RADIX_BITS: u32 = 8;

/// Number of buckets per pass. `1 << RADIX_BITS`.
pub const RADIX_BUCKETS: u32 = 1 << RADIX_BITS;

/// Number of passes for a 32-bit key. `32 / RADIX_BITS`.
pub const RADIX_PASSES: u32 = 32 / RADIX_BITS;

/// Threads per workgroup. Picked as a multiple of `RADIX_BUCKETS` so
/// each thread cleanly owns one bucket during the histogram-clear and
/// prefix-sum phases. 256 also matches the elevated compute invocation
/// limit configured by the engine (#258).
pub const SORT_WORKGROUP_SIZE: u32 = 256;

/// Items processed per workgroup tile. Smaller = more parallelism but
/// more decoupled-lookback chain hops; larger = fewer hops but more
/// register pressure per workgroup. 3072 = 12 items per thread × 256
/// threads, the sweet spot from the original paper for u32 keys.
pub const ITEMS_PER_TILE: u32 = SORT_WORKGROUP_SIZE * 12;

/// Status-descriptor flag bits. Packed into the high 2 bits of a u32
/// so the low 30 bits carry the partition's prefix value.
///
/// Flag values:
/// - `0` (INVALID): partition has not yet computed its local aggregate.
/// - `1` (AGGREGATE): local aggregate is published; lookback can read it
///   but must continue walking backwards to find a complete prefix.
/// - `2` (PREFIX): full inclusive prefix is published; lookback stops
///   here and uses this value as the base for downstream partitions.
pub const FLAG_INVALID: u32 = 0;
pub const FLAG_AGGREGATE: u32 = 1;
pub const FLAG_PREFIX: u32 = 2;

/// Per-pass uniform configuration. One instance per pass dispatch.
///
/// `pass_shift` is the bit-position of the current pass's radix digit
/// (`pass_shift = pass_index * RADIX_BITS`); the shader extracts the
/// digit with `(key >> pass_shift) & (RADIX_BUCKETS - 1)`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug, PartialEq)]
pub struct OnesweepConfig {
    /// Number of items being sorted. Threads with global index `>=`
    /// this early-out.
    pub count: u32,
    /// Bit-position of the current pass's radix digit.
    pub pass_shift: u32,
    /// Number of partitions (workgroups) for this dispatch.
    /// `partition_count = ceil(count / ITEMS_PER_TILE)`.
    pub partition_count: u32,
    /// Padding so the uniform block is 16-byte aligned (std140 / std430
    /// uniform requirement).
    pub _pad: u32,
}

impl OnesweepConfig {
    pub fn new(count: u32, pass_index: u32) -> Self {
        let partition_count = count.div_ceil(ITEMS_PER_TILE);
        Self {
            count,
            pass_shift: pass_index * RADIX_BITS,
            partition_count,
            _pad: 0,
        }
    }
}

/// Helper: bytes occupied by the global histogram buffer for the full
/// sort (all passes share one buffer, indexed by `pass_index *
/// RADIX_BUCKETS + bucket`).
pub const fn global_histogram_size_bytes() -> u64 {
    (RADIX_PASSES as u64) * (RADIX_BUCKETS as u64) * 4
}

/// Helper: bytes for the per-partition status descriptors of one pass.
/// One u32 per (partition × bucket) — the lookback walks backwards
/// through these.
pub fn partition_descriptors_size_bytes(partition_count: u32) -> u64 {
    (partition_count as u64) * (RADIX_BUCKETS as u64) * 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radix_constants_consistent() {
        assert_eq!(RADIX_BUCKETS, 256);
        assert_eq!(RADIX_PASSES, 4);
        assert_eq!(RADIX_PASSES * RADIX_BITS, 32);
    }

    #[test]
    fn workgroup_size_aligned_to_buckets() {
        // Per the histogram-clear and prefix-sum phases of onesweep,
        // each thread cleanly owns one bucket — workgroup size must
        // equal RADIX_BUCKETS.
        assert_eq!(SORT_WORKGROUP_SIZE, RADIX_BUCKETS);
    }

    #[test]
    fn items_per_tile_is_workgroup_multiple() {
        assert_eq!(ITEMS_PER_TILE % SORT_WORKGROUP_SIZE, 0);
        assert_eq!(ITEMS_PER_TILE / SORT_WORKGROUP_SIZE, 12);
    }

    #[test]
    fn config_layout_is_16_bytes() {
        // Must match the uniform alignment in WGSL.
        assert_eq!(std::mem::size_of::<OnesweepConfig>(), 16);
    }

    #[test]
    fn config_pass_shift_is_correct() {
        let c0 = OnesweepConfig::new(1000, 0);
        let c1 = OnesweepConfig::new(1000, 1);
        let c3 = OnesweepConfig::new(1000, 3);
        assert_eq!(c0.pass_shift, 0);
        assert_eq!(c1.pass_shift, 8);
        assert_eq!(c3.pass_shift, 24);
    }

    #[test]
    fn config_partition_count_round_up() {
        // Exactly one tile.
        let c = OnesweepConfig::new(ITEMS_PER_TILE, 0);
        assert_eq!(c.partition_count, 1);
        // One item more — needs two tiles.
        let c = OnesweepConfig::new(ITEMS_PER_TILE + 1, 0);
        assert_eq!(c.partition_count, 2);
        // Empty — zero partitions; the caller is responsible for
        // skipping the dispatch when count == 0.
        let c = OnesweepConfig::new(0, 0);
        assert_eq!(c.partition_count, 0);
    }

    #[test]
    fn global_histogram_size_is_4kb() {
        // 4 passes × 256 buckets × 4 bytes = 4096 bytes.
        assert_eq!(global_histogram_size_bytes(), 4096);
    }

    #[test]
    fn partition_descriptors_scale_with_count() {
        // 100 partitions × 256 buckets × 4 bytes = 102 400 bytes.
        assert_eq!(partition_descriptors_size_bytes(100), 102_400);
    }

    #[test]
    fn flag_values_are_distinct_and_low_bits() {
        // Flags pack into the high 2 bits of a u32 — must fit in 2 bits.
        assert!(FLAG_INVALID < 4);
        assert!(FLAG_AGGREGATE < 4);
        assert!(FLAG_PREFIX < 4);
        assert_ne!(FLAG_INVALID, FLAG_AGGREGATE);
        assert_ne!(FLAG_AGGREGATE, FLAG_PREFIX);
    }
}

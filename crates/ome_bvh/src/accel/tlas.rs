//! TLAS topology helpers — chunk-leaf encoding + lazy rebuild policy.
//!
//! The traversal contract: each TLAS leaf's `right_or_count` is a
//! tagged `chunk_idx`:
//!
//! - bit 31 — `BVH_LEAF_FLAG` (mirrors the BLAS leaf discriminator).
//! - bit 30 — [`TLAS_DEAD_FLAG`](super::TLAS_DEAD_FLAG): the slot was
//!   evicted, the lazy compactor hasn't rebuilt yet.
//! - bits 0..=29 — chunk index into `chunk_descriptors`.
//!
//! Streaming flow lives in `streaming.rs`; this module owns the
//! encoding primitives so the WGSL header and the CPU `BvhNode`
//! writer agree on the same constants.

use crate::accel::{TLAS_CHUNK_IDX_MASK, TLAS_DEAD_FLAG};
use crate::node::BVH_LEAF_FLAG;

/// Pack a live TLAS leaf's `right_or_count`.
#[inline]
pub fn encode_live(chunk_idx: u32) -> u32 {
    debug_assert!(chunk_idx & !TLAS_CHUNK_IDX_MASK == 0);
    chunk_idx | BVH_LEAF_FLAG
}

/// Pack an evicted TLAS leaf's `right_or_count`. Traversal short-
/// circuits without descending into the BLAS pool.
#[inline]
pub fn encode_dead(chunk_idx: u32) -> u32 {
    debug_assert!(chunk_idx & !TLAS_CHUNK_IDX_MASK == 0);
    chunk_idx | BVH_LEAF_FLAG | TLAS_DEAD_FLAG
}

/// Read the `chunk_idx` out of an encoded TLAS leaf payload.
#[inline]
pub fn decode_chunk_idx(right_or_count: u32) -> u32 {
    right_or_count & TLAS_CHUNK_IDX_MASK
}

/// `true` if the encoded TLAS leaf payload is dead-skip.
#[inline]
pub fn is_dead(right_or_count: u32) -> bool {
    right_or_count & TLAS_DEAD_FLAG != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_live_leaf() {
        let payload = encode_live(42);
        assert!(payload & BVH_LEAF_FLAG != 0);
        assert!(!is_dead(payload));
        assert_eq!(decode_chunk_idx(payload), 42);
    }

    #[test]
    fn round_trip_dead_leaf() {
        let payload = encode_dead(7);
        assert!(payload & BVH_LEAF_FLAG != 0);
        assert!(is_dead(payload));
        assert_eq!(decode_chunk_idx(payload), 7);
    }

    #[test]
    fn live_and_dead_distinguishable() {
        let live = encode_live(99);
        let dead = encode_dead(99);
        assert_ne!(live, dead);
        assert_eq!(decode_chunk_idx(live), decode_chunk_idx(dead));
    }
}

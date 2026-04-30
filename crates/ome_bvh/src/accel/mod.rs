//! `OmeAccel` — TLAS+BLAS pool-based acceleration structure for
//! planet-scale streaming (issue #360).
//!
//! Replaces the single global BVH that pre-#360 sat under
//! `RayMarchRenderer`. The pool layout is the standard answer in every
//! shipped planet-scale renderer (Nanite, bevy_pbr meshlets, Niagara,
//! Star Citizen Object Containers): contiguous global pools indexed by
//! 32-bit handles, per-instance descriptors, GPU-driven traversal in a
//! single compute pass.
//!
//! # Module layout
//!
//! - [`descriptor`] — `ChunkDescriptor` (64 B) + `TlasUniforms` (16 B).
//! - [`error`] — `AccelCaps` + `AccelError`.
//! - [`pool`] — `FreeListPool` for the byte pools.
//! - [`tlas`] — TLAS topology + leaf encoding (introduced in the
//!   follow-up commit that wires `OmeAccel::insert_chunk`).
//! - [`streaming`] — `insert_chunk` / `remove_chunk` / `refit_chunk`
//!   (introduced in the follow-up commit that lights up the streaming
//!   API).
//!
//! Each file stays under 400 LoC per the workspace's monolith rule.
//!
//! # TLAS leaf encoding
//!
//! TLAS leaves repurpose `BvhNode.right_or_count` as a tagged
//! `chunk_idx`. Two flag bits live in the high 31..=30 range so the
//! existing `BVH_LEAF_FLAG` discriminator stays compatible with the
//! generic traversal library:
//!
//! - bit 31 (`BVH_LEAF_FLAG`) — set on every TLAS leaf, exactly as
//!   for BLAS leaves.
//! - bit 30 (`TLAS_DEAD_FLAG`) — set when the chunk has been evicted
//!   but the lazy compactor has not yet rebuilt the TLAS topology.
//!   Traversal skips dead leaves without touching the BLAS pool.
//! - bits 0..=29 (`TLAS_CHUNK_IDX_MASK`) — `chunk_idx` into
//!   `chunk_descriptors`. Caps `MAX_CHUNKS` at `2^30 - 1` (≈ 1 G);
//!   the default cap of `1024` lives well below.

pub mod buffers;
pub mod descriptor;
pub mod error;
pub mod pool;
pub mod state;
pub mod streaming;
pub mod tlas;

pub use buffers::AccelBuffers;
pub use descriptor::{ChunkDescriptor, TlasUniforms};
pub use error::{AccelCaps, AccelError};
pub use pool::{FragmentationMetrics, FreeListPool, FreeRange};
pub use state::{ChunkBvhHandle, ChunkKey, OmeAccel};
pub use streaming::{ChunkInsert, ChunkRefit};

/// Set on TLAS leaves whose chunk has been evicted. Traversal skips
/// without descending into the BLAS pool.
pub const TLAS_DEAD_FLAG: u32 = 1u32 << 30;

/// Bits 0..=29 of a TLAS leaf's `right_or_count`: the chunk index into
/// `chunk_descriptors`.
pub const TLAS_CHUNK_IDX_MASK: u32 = 0x3FFF_FFFF;

/// Hard upper bound on `AccelCaps::max_chunks` set by the TLAS leaf
/// encoding (bit 30 reserved for `TLAS_DEAD_FLAG`, bit 31 by
/// `BVH_LEAF_FLAG`).
pub const MAX_CHUNKS_LIMIT: u32 = TLAS_CHUNK_IDX_MASK;

/// TLAS traversal stack depth. Deep enough for `2^32` leaves
/// worst-case (the encoding caps at `2^30`, so 32 frames are
/// comfortably more than required) and tight enough to fit in
/// registers on Steam Deck-class hardware.
pub const MAX_TLAS_STACK: u32 = 32;

/// BLAS traversal stack depth. Same reasoning as
/// [`MAX_TLAS_STACK`].
pub const MAX_BLAS_STACK: u32 = 32;

/// `inserts + removes` accumulated since the last full TLAS rebuild
/// before the lazy compactor switches from incremental refit to a
/// full LBVH rebuild. Initial value picked from the issue body;
/// profile-driven.
pub const TLAS_REBUILD_THRESHOLD: u32 = 16;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::BVH_LEAF_FLAG;

    #[test]
    fn tlas_flag_bits_distinct_from_bvh_leaf_flag() {
        assert_ne!(TLAS_DEAD_FLAG, BVH_LEAF_FLAG);
        assert_eq!(TLAS_DEAD_FLAG & BVH_LEAF_FLAG, 0);
    }

    #[test]
    fn tlas_chunk_idx_mask_covers_low_30_bits() {
        assert_eq!(TLAS_CHUNK_IDX_MASK & BVH_LEAF_FLAG, 0);
        assert_eq!(TLAS_CHUNK_IDX_MASK & TLAS_DEAD_FLAG, 0);
        assert_eq!(
            TLAS_CHUNK_IDX_MASK | BVH_LEAF_FLAG | TLAS_DEAD_FLAG,
            0xFFFF_FFFF
        );
    }

    #[test]
    fn max_chunks_default_below_encoding_limit() {
        assert!(AccelCaps::default().max_chunks <= MAX_CHUNKS_LIMIT);
        assert!(AccelCaps::TEST.max_chunks <= MAX_CHUNKS_LIMIT);
    }
}

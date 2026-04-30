//! Handles + per-slot CPU mirror for `OmeAccel`. Lives in its own
//! file so `state/mod.rs` stays under the workspace's 400-LoC monolith
//! cap.

use crate::accel::descriptor::ChunkDescriptor;

/// Opaque identifier for a chunk currently resident in the pool.
/// Returned by `insert_chunk` and consumed by `remove_chunk` /
/// `refit_chunk`. Callers that key by world-space coordinates encode
/// to a [`ChunkKey`] before insertion (a `u64` is sufficient for
/// signed `i20` × 3 axes — ~16 km radius at 16 m chunks).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ChunkBvhHandle {
    pub chunk_idx: u32,
}

/// CPU-side stable key the streaming layer uses to identify a chunk.
/// Encoding is the caller's responsibility — the pool only requires
/// `Hash + Eq + Copy`. A `u64` covers planet-scale signed `i20` × 3
/// axes with ~16 bits to spare for a generation counter.
pub type ChunkKey = u64;

/// Entry in the CPU side of the pool. Mirrors
/// `chunk_descriptors[chunk_idx]` plus the streaming bookkeeping the
/// GPU never sees.
///
/// `sorted_indices[k]` is the original-position of the BLAS leaf at
/// sorted position `k`. The value is the absolute pool index
/// (`first_primitive + original_local_index`) so the WGSL traversal
/// can read primitives directly without per-chunk fixup.
#[derive(Clone, Debug, Default)]
pub(crate) struct ChunkSlot {
    pub(crate) descriptor: ChunkDescriptor,
    pub(crate) live: bool,
    /// Stored for audit / debug reflection — the canonical lookup
    /// path is `coord_to_idx`. Kept on the slot so a future
    /// chunk-by-chunk diff dump has the key alongside the offsets.
    #[allow(dead_code)]
    pub(crate) key: ChunkKey,
    pub(crate) sorted_indices: Vec<u32>,
}

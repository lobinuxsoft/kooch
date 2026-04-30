//! Handles + per-slot CPU mirror for `OmeAccel`. Lives in its own
//! file so `state/mod.rs` stays under the workspace's 400-LoC monolith
//! cap.

use crate::accel::descriptor::ChunkDescriptor;
use crate::leaf::LeafAabb;
use crate::node::BvhNode;

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
/// # CPU mirrors
///
/// `cpu_bvh_nodes` and `cpu_leaf_aabbs` shadow the corresponding GPU
/// pool slices for this chunk. They're written synchronously alongside
/// the `Queue::write_buffer` calls during `insert_chunk` / `refit_chunk`
/// — this is **maintained mirror**, not GPU readback, so the
/// no-readback-in-hot-path constraint stays intact.
///
/// CPU consumers (today: `ome_physics::broadphase`; tomorrow: any
/// CPU-side narrowphase or editor inspector) walk these directly via
/// `OmeAccel::for_each_overlapping_cpu`. Memory cost is
/// `O(live_primitives)` — scales with the scene, not with the cap.
///
/// `sorted_indices[k]` is the **absolute pool primitive index** of the
/// BLAS leaf at sorted position `k` (= `first_primitive + original_i`).
/// Used by `refit_chunk_slice_only` for the topology-preserving fast
/// path; the WGSL traversal indexes `node.left` directly so the
/// shader never reads this array.
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
    /// Karras BVH nodes for this chunk's BLAS, in pool layout (post-
    /// pass applied so leaf nodes carry absolute pool primitive
    /// indices in `node.left`).
    pub(crate) cpu_bvh_nodes: Vec<BvhNode>,
    /// Leaf AABBs in **original-input order**, matching how
    /// `leaf_aabbs_pool` is laid out on the GPU side. Indexed by
    /// `(absolute_primitive_index - descriptor.first_primitive)`.
    pub(crate) cpu_leaf_aabbs: Vec<LeafAabb>,
}

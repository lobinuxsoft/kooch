//! Data-transfer objects for the `OmeAccel` streaming API.
//!
//! Borrowed-slice inputs only — the pool copies into its GPU buffers
//! and never aliases the caller's allocations. Lives in its own
//! file so `streaming/mod.rs` stays under the workspace's 400-LoC
//! monolith cap.

use crate::accel::state::ChunkKey;
use crate::leaf::LeafAabb;

/// Inputs for one `insert_chunk` call. Borrowed slices — the pool
/// copies into its GPU buffers and never aliases the caller's
/// allocations.
pub struct ChunkInsert<'a> {
    /// Streaming-layer-stable key. Used to look the chunk back up
    /// from `remove_chunk` / `refit_chunk`.
    pub key: ChunkKey,
    /// Per-primitive `LeafAabb`. `len() = primitive_count` for this
    /// chunk. `aabb_min` / `aabb_max` already inflated by the
    /// per-role envelope.
    pub leaf_aabbs: &'a [LeafAabb],
    /// Per-primitive opaque payload. Length must equal
    /// `leaf_aabbs.len() * primitive_stride`.
    pub primitives_bytes: &'a [u8],
    /// Conservative envelope used by the TLAS culling — typically
    /// `max(k_add, k_sub, k_int)` over this chunk's primitives.
    pub max_smoothness_radius: f32,
}

/// Inputs for one `refit_chunk` call. Same primitive count as the
/// chunk's last `insert_chunk` — the topology is preserved.
pub struct ChunkRefit<'a> {
    pub key: ChunkKey,
    pub leaf_aabbs: &'a [LeafAabb],
    pub primitives_bytes: &'a [u8],
    pub max_smoothness_radius: f32,
}

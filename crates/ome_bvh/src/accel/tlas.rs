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

use bytemuck::cast_slice;
use glam::Vec3;
use std::mem::size_of;

use crate::aabb::Aabb;
use crate::accel::state::OmeAccel;
use crate::accel::{TLAS_CHUNK_IDX_MASK, TLAS_DEAD_FLAG};
use crate::bvh::Bvh;
use crate::node::{BVH_LEAF_FLAG, BvhNode};

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

/// Full TLAS rebuild — invoked by `OmeAccel::update_gpu` whenever
/// `tlas_dirty_count > 0`. Collects every live chunk's inflated
/// AABB, builds a Karras topology over `(chunk_idx, aabb)` pairs,
/// rewrites each leaf's `right_or_count` to encode the live-leaf
/// payload, and uploads the result in a single
/// `Queue::write_buffer` call.
///
/// Writing always happens — even an empty pool clears the GPU buffer
/// to the canonical zeroed state so a stale TLAS from a previous
/// frame can never bleed through.
pub(crate) fn rebuild(accel: &mut OmeAccel, queue: &wgpu::Queue) {
    let live_chunks: Vec<(u32, Aabb)> = accel
        .slots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            if !s.live {
                return None;
            }
            let aabb = Aabb::new(
                Vec3::from(s.descriptor.aabb_min),
                Vec3::from(s.descriptor.aabb_max),
            );
            Some((i as u32, aabb))
        })
        .collect();

    let n = live_chunks.len();
    if n == 0 {
        // Empty pool: zero the first node so any stale traversal sees
        // an out-of-bounds AABB and bails out immediately.
        let zero = BvhNode::default();
        queue.write_buffer(&accel.buffers.tlas_nodes, 0, bytemuck::bytes_of(&zero));
        return;
    }

    let total_nodes = if n == 1 { 1 } else { 2 * n - 1 };
    let mut nodes_scratch = vec![BvhNode::default(); total_nodes];
    let mut leaves_scratch = vec![0u32; n];
    Bvh::<u32>::build_into(live_chunks, &mut nodes_scratch, &mut leaves_scratch);

    // Override each leaf's `right_or_count` to encode the chunk index.
    // The builder leaves `right_or_count = 1 | BVH_LEAF_FLAG` — we
    // replace `1` with the actual chunk_idx so the WGSL traversal
    // resolves a leaf to a chunk descriptor in one fetch.
    let leaf_offset = n.saturating_sub(1);
    for k in 0..n {
        let chunk_idx = leaves_scratch[k];
        nodes_scratch[leaf_offset + k].left = 0;
        nodes_scratch[leaf_offset + k].right_or_count = encode_live(chunk_idx);
    }

    queue.write_buffer(
        &accel.buffers.tlas_nodes,
        0,
        cast_slice(&nodes_scratch[..total_nodes]),
    );
    debug_assert!(
        total_nodes * size_of::<BvhNode>() <= 2 * accel.caps.max_chunks as usize * size_of::<BvhNode>(),
        "TLAS rebuild overflowed pre-allocated tlas_nodes buffer",
    );
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

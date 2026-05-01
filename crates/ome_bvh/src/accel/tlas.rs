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

use glam::Vec3;

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
/// `tlas_dirty_count > 0`. As of epic #370 PR-1 the production path
/// is GPU-driven via [`crate::gpu::tlas_lbvh::TlasGpuBuilder`]:
/// morton + sort + leaves + internal + aabb passes recorded into the
/// caller's encoder, no CPU readback in the hot path.
///
/// The CPU mirror (`accel.cpu_tlas_nodes`) is rebuilt eagerly via the
/// legacy [`Bvh::<u32>::build_into`] path so `for_each_overlapping_cpu`
/// stays correct without an extra signature thread (broadphase queries
/// take `&OmeAccel`, ruling out a `&mut self` lazy refresh hook).
///
/// Empty pool (`live_chunk_count == 0`) writes a zeroed sentinel into
/// `tlas_nodes[0]` so any stale traversal sees an out-of-bounds AABB
/// and bails out — same legacy semantics, queue-side only (no encoder
/// recording needed for the sentinel).
pub(crate) fn rebuild(
    accel: &mut OmeAccel,
    encoder: &mut wgpu::CommandEncoder,
    queue: &wgpu::Queue,
) {
    let n = accel.live_chunk_count();
    if n == 0 {
        let zero = BvhNode::default();
        queue.write_buffer(&accel.buffers.tlas_nodes, 0, bytemuck::bytes_of(&zero));
        accel.cpu_tlas_nodes.clear();
        return;
    }

    // Lazy init of the GPU pipelines on first rebuild — keeps
    // CPU-only test fixtures (downlevel_defaults limits) usable until
    // they actually need a rebuild.
    accel.ensure_tlas_builder();

    // GPU dispatch — records morton + sort + leaves + internal + aabb
    // into the caller's encoder. Cero CPU readback. Split borrows
    // pull `tlas_builder`, `buffers`, and `device` independently.
    let cpu_descriptors = accel.live_chunk_descriptors();
    let builder = accel.tlas_builder.as_ref().expect("ensure_tlas_builder ran");
    builder.dispatch_rebuild(
        &accel.device,
        queue,
        encoder,
        &accel.buffers,
        &cpu_descriptors,
        n,
    );

    // CPU mirror eager rebuild (legacy semantics). Keeps
    // `for_each_overlapping_cpu` consumers correct without changing
    // their borrow shape. NLL releases the `builder` shared borrow at
    // the dispatch above so the `&mut accel` re-borrow here is fine.
    rebuild_cpu_mirror(accel, &cpu_descriptors);
}

/// CPU-side rebuild of `cpu_tlas_nodes` using the legacy
/// [`Bvh::<u32>::build_into`] path. Eagerly mirrors the GPU result
/// shape (with the chunk_idx-encoded leaf payload via [`encode_live`])
/// so CPU consumers byte-match what the GPU pipeline produces.
fn rebuild_cpu_mirror(accel: &mut OmeAccel, cpu_descriptors: &[crate::accel::descriptor::ChunkDescriptor]) {
    let live: Vec<(u32, Aabb)> = accel
        .slots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            if !s.live {
                return None;
            }
            Some((
                i as u32,
                Aabb::new(Vec3::from(s.descriptor.aabb_min), Vec3::from(s.descriptor.aabb_max)),
            ))
        })
        .collect();
    debug_assert_eq!(live.len(), cpu_descriptors.len());
    let n = live.len();
    let total = if n == 1 { 1 } else { 2 * n - 1 };
    let mut nodes = vec![BvhNode::default(); total];
    let mut leaves = vec![0u32; n];
    Bvh::<u32>::build_into(live, &mut nodes, &mut leaves);

    let leaf_offset = n.saturating_sub(1);
    for k in 0..n {
        let chunk_idx = leaves[k];
        nodes[leaf_offset + k].left = 0;
        nodes[leaf_offset + k].right_or_count = encode_live(chunk_idx);
    }
    accel.cpu_tlas_nodes = nodes;
}

/// Legacy CPU-only rebuild preserved for commit 10's ground-truth
/// comparison. Does NOT touch the GPU buffer — only populates a fresh
/// `Vec<BvhNode>` matching what `tlas_nodes` should contain after the
/// GPU dispatch settles. Returns `(nodes, total_node_count)`; `nodes`
/// is sized `2 * n` (rounded) but only `total_node_count` are valid.
#[cfg(test)]
pub(crate) fn rebuild_cpu_legacy(accel: &OmeAccel) -> (Vec<BvhNode>, usize) {
    let live: Vec<(u32, Aabb)> = accel
        .slots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            if !s.live {
                return None;
            }
            Some((
                i as u32,
                Aabb::new(Vec3::from(s.descriptor.aabb_min), Vec3::from(s.descriptor.aabb_max)),
            ))
        })
        .collect();
    let n = live.len();
    if n == 0 {
        return (vec![BvhNode::default()], 1);
    }
    let total = if n == 1 { 1 } else { 2 * n - 1 };
    let mut nodes = vec![BvhNode::default(); total];
    let mut leaves = vec![0u32; n];
    Bvh::<u32>::build_into(live, &mut nodes, &mut leaves);

    let leaf_offset = n.saturating_sub(1);
    for k in 0..n {
        let chunk_idx = leaves[k];
        nodes[leaf_offset + k].left = 0;
        nodes[leaf_offset + k].right_or_count = encode_live(chunk_idx);
    }
    (nodes, total)
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

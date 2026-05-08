//! Topology-preserving refit primitives. The owning
//! [`Bvh::refit_in_place`](super::types::Bvh::refit_in_place) variant
//! lives in `builder.rs`; here we keep the slice-destination
//! [`refit_slice_in_place`] (re-exported from `crate::bvh`) plus the
//! shared [`propagate_aabb`] helper used by both the builder and the
//! refit path.

use glam::Vec3;

use crate::aabb::Aabb;
use crate::leaf::LeafAabb;
use crate::node::BvhNode;

/// Refit the leaf AABBs of an externally-owned `nodes` slice, then
/// propagate up. Pool-friendly variant of
/// [`Bvh::refit_in_place`] — writes through a `&mut [BvhNode]`
/// destination so `OmeAccel` can refit one chunk's BLAS slice without
/// rebuilding the whole pool.
///
/// [`Bvh::refit_in_place`]: super::types::Bvh::refit_in_place
///
/// # Caller invariants
///
/// - `nodes.len() >= 2 * n - 1` (or `nodes.len() >= 1` when `n == 1`).
/// - `leaf_aabbs.len() >= n`. `leaf_aabbs[i]` is the world-space
///   AABB of the original-input position `i`.
/// - `sorted_indices.len() >= n`. `sorted_indices[k]` is the
///   original-input position of the leaf at sorted position `k`,
///   i.e. the leaf node at `nodes[(n - 1) + k]`.
///
/// Mirrors [`Bvh::refit_in_place`] semantics 1:1 — only the
/// destination type changes.
pub fn refit_slice_in_place(
    nodes: &mut [BvhNode],
    n: usize,
    leaf_aabbs: &[LeafAabb],
    sorted_indices: &[u32],
) {
    debug_assert!(
        leaf_aabbs.len() >= n,
        "refit_slice_in_place: leaf_aabbs.len() = {} < n = {}",
        leaf_aabbs.len(),
        n,
    );
    debug_assert!(
        sorted_indices.len() >= n,
        "refit_slice_in_place: sorted_indices.len() = {} < n = {}",
        sorted_indices.len(),
        n,
    );
    if n == 0 {
        return;
    }
    let total_nodes = if n == 1 { 1 } else { 2 * n - 1 };
    debug_assert!(
        nodes.len() >= total_nodes,
        "refit_slice_in_place: nodes.len() = {} < required {}",
        nodes.len(),
        total_nodes,
    );
    let leaf_offset = n.saturating_sub(1);
    for k in 0..n {
        let original = sorted_indices[k] as usize;
        let new_aabb = &leaf_aabbs[original];
        nodes[leaf_offset + k].aabb_min = new_aabb.aabb_min;
        nodes[leaf_offset + k].aabb_max = new_aabb.aabb_max;
    }
    if n > 1 {
        propagate_aabb(&mut nodes[..total_nodes], 0);
    }
}

/// Recursive post-order DFS that propagates child AABBs into their
/// parent. Returns the AABB of the subtree rooted at `idx` so the
/// parent can read it without re-fetching from the array.
///
/// Stack depth is bounded by tree depth (≤ ⌈log₂ N⌉ + small constant
/// for unbalanced trees on duplicate codes); safe for any practical N.
pub(super) fn propagate_aabb(nodes: &mut [BvhNode], idx: usize) -> Aabb {
    let node = nodes[idx];
    if node.is_leaf() {
        return Aabb::new(Vec3::from(node.aabb_min), Vec3::from(node.aabb_max));
    }
    let left = node.left as usize;
    let right = node.right_child() as usize;
    let left_aabb = propagate_aabb(nodes, left);
    let right_aabb = propagate_aabb(nodes, right);
    let combined = left_aabb.union(&right_aabb);
    nodes[idx].aabb_min = combined.min.into();
    nodes[idx].aabb_max = combined.max.into();
    combined
}

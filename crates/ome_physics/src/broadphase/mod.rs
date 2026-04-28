//! CPU broadphase over the engine-shared BVH.
//!
//! [`BroadphasePairs::collect`] walks every leaf flagged
//! [`IS_COLLIDER`] in the [`SharedBvhState`]'s CPU mirror, queries the
//! BVH for AABB-overlapping leaves via [`Bvh::for_each_aabb`], and
//! records canonical `(low, high)` entity-id pairs deduplicated across
//! both sides of the symmetric query.
//!
//! # Why CPU-first
//!
//! Narrowphase (#40) is still CPU. Running broadphase on the GPU would
//! force a per-frame readback of the candidate-pair list — exactly the
//! shape PR-3..PR-5 spent effort designing AWAY from. When narrowphase
//! lands on the GPU, the GPU broadphase replaces the body of this
//! method without changing the API: still `BroadphasePairs::collect(&shared)`,
//! still `(EntityId, EntityId)` pairs.
//!
//! This consumer is the second to use the multi-consumer BVH after
//! the raymarch (#115 PR-4) — it satisfies AC 116 of the parent issue
//! ("múltiples sistemas usan la misma estructura"). The CPU mirror it
//! reads is populated by `SharedBvhState::poll_swap` from the GPU
//! build's already-paid-for readback; broadphase pays nothing extra
//! for the BVH itself.

use ome_bvh::{Aabb, Bvh, IS_COLLIDER, LeafAabb, SharedBvhState};

/// Collision pair — two entity ids whose AABBs overlap. Stored with
/// `low <= high` so duplicate detection is a single sort + dedup.
pub type CollisionPair = (u32, u32);

/// Result of a broadphase pass: the deduplicated set of collider
/// AABB-overlap candidate pairs. Ownership lives in the consuming
/// system; narrowphase walks `pairs()` to refine.
#[derive(Default, Debug)]
pub struct BroadphasePairs {
    pairs: Vec<CollisionPair>,
}

impl BroadphasePairs {
    /// Run a CPU broadphase over the [`SharedBvhState`]'s CPU mirror.
    /// Returns an empty result when the mirror has not yet been
    /// populated (no successful build has resolved).
    ///
    /// CPU traversal cost is `O(C · log N)` for `C` colliders in a tree
    /// of `N` leaves, ignoring per-pair work. Brute force `O(N²)`
    /// stays as the reference for the consistency test, not the
    /// production path.
    pub fn collect(shared: &SharedBvhState) -> Self {
        match (shared.current_cpu_bvh(), shared.current_cpu_leaf_aabbs()) {
            (Some(bvh), Some(leaf_aabbs)) => Self::from_cpu_mirror(bvh, leaf_aabbs),
            _ => Self::default(),
        }
    }

    /// Run a CPU broadphase directly over a CPU `Bvh<u32>` + per-leaf
    /// metadata in original input order. Bypasses [`SharedBvhState`]
    /// for the pure-CPU test path and for tooling that owns its own
    /// CPU bvh; production callers want [`Self::collect`].
    ///
    /// `bvh.leaves[k]` is the original-input position of the leaf at
    /// sorted position `k`; that position indexes `leaf_aabbs`. The
    /// loop iterates `leaf_aabbs` directly to skip the indirection on
    /// the outer query side.
    pub fn from_cpu_mirror(bvh: &Bvh<u32>, leaf_aabbs: &[LeafAabb]) -> Self {
        // Worst-case `O(C²)` pair count — but the tree prunes most of
        // it for spatially-distributed scenes. We sort + dedup at the
        // end, so per-pair allocation is the only steady-state cost.
        let mut pairs: Vec<CollisionPair> = Vec::new();
        for (i, la) in leaf_aabbs.iter().enumerate() {
            if la.flags & IS_COLLIDER == 0 {
                continue;
            }
            let aabb_i = Aabb::new(la.aabb_min.into(), la.aabb_max.into());
            let entity_i = la.entity_id;
            bvh.for_each_aabb(aabb_i, |&original_pos| {
                let j = original_pos as usize;
                // Skip self-pairs and out-of-range readbacks (defensive
                // — leaf_aabbs.len() == bvh.leaf_count() by invariant).
                if j == i || j >= leaf_aabbs.len() {
                    return;
                }
                let lj = &leaf_aabbs[j];
                if lj.flags & IS_COLLIDER == 0 {
                    return;
                }
                let entity_j = lj.entity_id;
                let pair = if entity_i <= entity_j {
                    (entity_i, entity_j)
                } else {
                    (entity_j, entity_i)
                };
                pairs.push(pair);
            });
        }
        // Each overlap shows up twice (once from i's query, once from
        // j's). Sort + dedup canonicalises to a single entry per pair.
        pairs.sort_unstable();
        pairs.dedup();
        Self { pairs }
    }

    /// Borrow the deduplicated pair list. Each entry is `(low, high)`
    /// by entity id.
    pub fn pairs(&self) -> &[CollisionPair] {
        &self.pairs
    }

    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}


#[cfg(test)]
mod bench;
#[cfg(test)]
mod tests;

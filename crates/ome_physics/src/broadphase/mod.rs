//! CPU broadphase over the OmeAccel TLAS+BLAS pool.
//!
//! [`BroadphasePairs::collect`] iterates every collider leaf across
//! every live chunk in the pool, queries the TLAS+BLAS for AABB-
//! overlapping leaves via `OmeAccel::for_each_overlapping_cpu`, and
//! records canonical `(low, high)` entity-id pairs deduplicated across
//! both sides of the symmetric query.
//!
//! Cross-chunk overlaps fall out for free: the TLAS descend visits
//! every chunk whose inflated AABB overlaps the query, and the per-
//! chunk BLAS descend filters down to the actual leaf candidates.
//! No special case for entities straddling two chunks.
//!
//! # Why CPU-first
//!
//! Narrowphase (#40) is still CPU. Running broadphase on the GPU
//! would force a per-frame readback of the candidate-pair list —
//! exactly the shape PR-3..PR-5 designed AWAY from. When narrowphase
//! moves to GPU compute, the broadphase will be replaced by a GPU
//! compute pass without changing the API on the consumer side: still
//! `BroadphasePairs::collect(&accel)`, still `(EntityId, EntityId)`
//! pairs.
//!
//! The CPU mirrors the broadphase reads (`OmeAccel.cpu_tlas_nodes`,
//! `slot.cpu_bvh_nodes`, `slot.cpu_leaf_aabbs`) are written
//! synchronously by the streaming layer — broadphase pays nothing
//! extra for the BVH itself.

use ome_bvh::{Aabb, IS_COLLIDER, OmeAccel};

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
    /// Run a CPU broadphase over an [`OmeAccel`] pool. Empty pool
    /// returns an empty result. Cross-chunk overlap is automatic via
    /// the TLAS descend in `OmeAccel::for_each_overlapping_cpu`.
    ///
    /// CPU traversal cost is `O(C · log N)` for `C` colliders in a pool
    /// with `N` total leaves. The overhead vs the legacy single-BVH
    /// path is one TLAS descend per query — `O(log num_chunks)` extra
    /// work per collider, dwarfed by the BLAS descend cost on
    /// realistic scenes.
    pub fn collect(accel: &OmeAccel) -> Self {
        let mut pairs: Vec<CollisionPair> = Vec::new();
        // First pass: collect (entity_id, aabb) for every collider
        // leaf across every live chunk. We can't borrow the slot's
        // leaf_aabbs while also issuing pool queries (the closure
        // captures `pairs` mutably), so snapshot the colliders first.
        let mut colliders: Vec<(u32, Aabb)> = Vec::new();
        for (_chunk_idx, leaves) in accel.iter_live_leaves() {
            for la in leaves {
                if la.flags & IS_COLLIDER == 0 {
                    continue;
                }
                colliders.push((
                    la.entity_id,
                    Aabb::new(la.aabb_min.into(), la.aabb_max.into()),
                ));
            }
        }
        // Second pass: for each collider, query the pool. Skip the
        // self-pair and any non-collider hit. Cross-chunk overlap is
        // handled by the TLAS descend automatically.
        for (entity_i, aabb_i) in &colliders {
            accel.for_each_overlapping_cpu(*aabb_i, |_, leaf_j| {
                if leaf_j.flags & IS_COLLIDER == 0 {
                    return;
                }
                let entity_j = leaf_j.entity_id;
                if entity_j == *entity_i {
                    return;
                }
                let pair = if *entity_i <= entity_j {
                    (*entity_i, entity_j)
                } else {
                    (entity_j, *entity_i)
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
mod tests;

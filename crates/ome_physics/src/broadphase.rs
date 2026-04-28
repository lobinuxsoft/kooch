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
mod tests {
    use super::*;
    use glam::Vec3;
    use ome_bvh::{IS_RAYMARCH, ROLE_RAYMARCH_ADD};
    use std::collections::HashSet;

    fn aabb_at(centre: Vec3, half: f32) -> Aabb {
        Aabb::from_centre(centre, Vec3::splat(half))
    }

    fn collider_leaf(centre: Vec3, half: f32, entity_id: u32) -> LeafAabb {
        let a = aabb_at(centre, half);
        LeafAabb {
            aabb_min: a.min.into(),
            flags: IS_COLLIDER,
            aabb_max: a.max.into(),
            entity_id,
        }
    }

    fn raymarch_only_leaf(centre: Vec3, half: f32, entity_id: u32) -> LeafAabb {
        let a = aabb_at(centre, half);
        LeafAabb {
            aabb_min: a.min.into(),
            flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
            aabb_max: a.max.into(),
            entity_id,
        }
    }

    fn brute_force_pairs(leaf_aabbs: &[LeafAabb]) -> HashSet<CollisionPair> {
        let mut out = HashSet::new();
        for (i, la) in leaf_aabbs.iter().enumerate() {
            if la.flags & IS_COLLIDER == 0 {
                continue;
            }
            let ai = Aabb::new(la.aabb_min.into(), la.aabb_max.into());
            for (_j, lb) in leaf_aabbs.iter().enumerate().skip(i + 1) {
                if lb.flags & IS_COLLIDER == 0 {
                    continue;
                }
                let aj = Aabb::new(lb.aabb_min.into(), lb.aabb_max.into());
                if ai.intersects_aabb(&aj) {
                    let (a, b) = (la.entity_id, lb.entity_id);
                    out.insert(if a <= b { (a, b) } else { (b, a) });
                }
            }
        }
        out
    }

    #[test]
    fn empty_inputs_yield_empty_pairs() {
        let bvh: Bvh<u32> = Bvh::empty();
        let leaf_aabbs: Vec<LeafAabb> = Vec::new();
        let pairs = BroadphasePairs::from_cpu_mirror(&bvh, &leaf_aabbs);
        assert!(pairs.is_empty());
    }

    #[test]
    fn single_collider_yields_no_pairs() {
        let leaf_aabbs = vec![collider_leaf(Vec3::ZERO, 0.5, 7)];
        let items: Vec<(u32, Aabb)> = leaf_aabbs
            .iter()
            .enumerate()
            .map(|(i, la)| (i as u32, Aabb::new(la.aabb_min.into(), la.aabb_max.into())))
            .collect();
        let bvh = Bvh::build(items);
        let pairs = BroadphasePairs::from_cpu_mirror(&bvh, &leaf_aabbs);
        assert!(pairs.is_empty());
    }

    #[test]
    fn two_overlapping_colliders_yield_one_pair() {
        let leaf_aabbs = vec![
            collider_leaf(Vec3::ZERO, 0.5, 10),
            collider_leaf(Vec3::splat(0.3), 0.5, 20),
        ];
        let items: Vec<(u32, Aabb)> = leaf_aabbs
            .iter()
            .enumerate()
            .map(|(i, la)| (i as u32, Aabb::new(la.aabb_min.into(), la.aabb_max.into())))
            .collect();
        let bvh = Bvh::build(items);
        let pairs = BroadphasePairs::from_cpu_mirror(&bvh, &leaf_aabbs);
        assert_eq!(pairs.pairs(), &[(10, 20)]);
    }

    #[test]
    fn two_disjoint_colliders_yield_no_pairs() {
        let leaf_aabbs = vec![
            collider_leaf(Vec3::ZERO, 0.5, 10),
            collider_leaf(Vec3::splat(10.0), 0.5, 20),
        ];
        let items: Vec<(u32, Aabb)> = leaf_aabbs
            .iter()
            .enumerate()
            .map(|(i, la)| (i as u32, Aabb::new(la.aabb_min.into(), la.aabb_max.into())))
            .collect();
        let bvh = Bvh::build(items);
        let pairs = BroadphasePairs::from_cpu_mirror(&bvh, &leaf_aabbs);
        assert!(pairs.is_empty());
    }

    #[test]
    fn raymarch_only_leaves_are_ignored() {
        // A raymarch-only leaf overlapping two colliders must NOT
        // produce raymarch↔collider pairs. Broadphase scopes itself
        // strictly to IS_COLLIDER ↔ IS_COLLIDER overlaps.
        let leaf_aabbs = vec![
            collider_leaf(Vec3::ZERO, 0.5, 10),
            raymarch_only_leaf(Vec3::splat(0.2), 0.5, 99),
            collider_leaf(Vec3::splat(0.4), 0.5, 20),
        ];
        let items: Vec<(u32, Aabb)> = leaf_aabbs
            .iter()
            .enumerate()
            .map(|(i, la)| (i as u32, Aabb::new(la.aabb_min.into(), la.aabb_max.into())))
            .collect();
        let bvh = Bvh::build(items);
        let pairs = BroadphasePairs::from_cpu_mirror(&bvh, &leaf_aabbs);
        // Only collider↔collider pair (10, 20). The raymarch-only
        // leaf 99 is invisible to broadphase even though spatially
        // overlapping both.
        assert_eq!(pairs.pairs(), &[(10, 20)]);
    }

    #[test]
    fn random_1000_colliders_match_brute_force() {
        // 1000 colliders distributed in a 10×10×10 grid with radius
        // 0.6 — enough overlap to exercise the BVH traversal pruning
        // but not so dense that brute force becomes meaningless.
        let mut rng_state = 0xC0DEC0DEu32;
        let mut rand = || {
            rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
            (rng_state >> 16) as f32 / 32768.0
        };
        let leaf_aabbs: Vec<LeafAabb> = (0..1000u32)
            .map(|i| {
                let p = Vec3::new(rand(), rand(), rand()) * 10.0;
                collider_leaf(p, 0.6, i)
            })
            .collect();
        let items: Vec<(u32, Aabb)> = leaf_aabbs
            .iter()
            .enumerate()
            .map(|(i, la)| (i as u32, Aabb::new(la.aabb_min.into(), la.aabb_max.into())))
            .collect();
        let bvh = Bvh::build(items);
        let pairs = BroadphasePairs::from_cpu_mirror(&bvh, &leaf_aabbs);
        let bvh_set: HashSet<CollisionPair> = pairs.pairs().iter().copied().collect();
        let brute = brute_force_pairs(&leaf_aabbs);
        assert_eq!(
            bvh_set, brute,
            "broadphase BVH pairs must match brute force O(N²) ground truth",
        );
        // Belt-and-suspenders: dedup invariant holds.
        assert_eq!(bvh_set.len(), pairs.len(), "duplicate pair leaked through");
    }

    #[test]
    fn dedup_canonicalises_low_high() {
        // Two overlapping colliders with `entity_id` chosen so the
        // smaller id is at the second leaf — verifies the pair is
        // emitted as `(small, large)` regardless of leaf order.
        let leaf_aabbs = vec![
            collider_leaf(Vec3::ZERO, 0.5, 99),
            collider_leaf(Vec3::splat(0.3), 0.5, 5),
        ];
        let items: Vec<(u32, Aabb)> = leaf_aabbs
            .iter()
            .enumerate()
            .map(|(i, la)| (i as u32, Aabb::new(la.aabb_min.into(), la.aabb_max.into())))
            .collect();
        let bvh = Bvh::build(items);
        let pairs = BroadphasePairs::from_cpu_mirror(&bvh, &leaf_aabbs);
        assert_eq!(pairs.pairs(), &[(5, 99)]);
    }
}

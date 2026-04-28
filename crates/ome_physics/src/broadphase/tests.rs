//! Correctness tests for [`super::BroadphasePairs`]. Helpers are
//! `pub(super)` so the sibling [`super::bench`] module shares the
//! fixtures without duplication.

use std::collections::HashSet;

use glam::Vec3;
use ome_bvh::{Aabb, Bvh, IS_COLLIDER, IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_ADD};

use super::{BroadphasePairs, CollisionPair};

pub(super) fn aabb_at(centre: Vec3, half: f32) -> Aabb {
    Aabb::from_centre(centre, Vec3::splat(half))
}

pub(super) fn collider_leaf(centre: Vec3, half: f32, entity_id: u32) -> LeafAabb {
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

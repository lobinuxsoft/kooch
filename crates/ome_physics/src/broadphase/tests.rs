//! Correctness tests for [`super::BroadphasePairs`] over `OmeAccel`.
//!
//! All tests `wgpu::Device`-gate via `skip_if_no_device` so the suite
//! still passes on adapter-less CI runs.

use std::collections::HashSet;

use glam::Vec3;
use ome_bvh::{
    Aabb, AccelCaps, ChunkInsert, IS_COLLIDER, IS_RAYMARCH, LeafAabb, OmeAccel,
    ROLE_RAYMARCH_ADD,
};

use super::{BroadphasePairs, CollisionPair};

fn skip_if_no_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(
        instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("ome_physics::broadphase::tests"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()?;
    Some((device, queue))
}

fn collider_leaf(centre: Vec3, half: f32, entity_id: u32) -> LeafAabb {
    let a = Aabb::from_centre(centre, Vec3::splat(half));
    LeafAabb {
        aabb_min: a.min.into(),
        flags: IS_COLLIDER,
        aabb_max: a.max.into(),
        entity_id,
    }
}

fn raymarch_only_leaf(centre: Vec3, half: f32, entity_id: u32) -> LeafAabb {
    let a = Aabb::from_centre(centre, Vec3::splat(half));
    LeafAabb {
        aabb_min: a.min.into(),
        flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
        aabb_max: a.max.into(),
        entity_id,
    }
}

/// Insert one chunk holding `leaves` into a fresh OmeAccel and return
/// the pool. Helper for the single-chunk tests below; the multi-chunk
/// AC5 lives in `crates/ome_render/tests/ac5_physics_cross_chunk.rs`.
fn pool_with_leaves(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    leaves: &[LeafAabb],
) -> OmeAccel {
    let mut accel = OmeAccel::new(device, AccelCaps::TEST, 16).unwrap();
    if leaves.is_empty() {
        accel.update_gpu_standalone(device, queue, 0.0, 0.0);
        return accel;
    }
    let prim_bytes = vec![0u8; 16 * leaves.len()];
    accel
        .insert_chunk(
            queue,
            ChunkInsert {
                key: 0,
                leaf_aabbs: leaves,
                primitives_bytes: &prim_bytes,
                max_smoothness_radius: 0.0,
            },
        )
        .unwrap();
    accel.update_gpu_standalone(device, queue, 0.0, 0.0);
    accel
}

fn brute_force_pairs(leaf_aabbs: &[LeafAabb]) -> HashSet<CollisionPair> {
    let mut out = HashSet::new();
    for (i, la) in leaf_aabbs.iter().enumerate() {
        if la.flags & IS_COLLIDER == 0 {
            continue;
        }
        let ai = Aabb::new(la.aabb_min.into(), la.aabb_max.into());
        for lb in leaf_aabbs.iter().skip(i + 1) {
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
fn empty_pool_yields_empty_pairs() {
    let Some((device, queue)) = skip_if_no_device() else { return; };
    let accel = pool_with_leaves(&device, &queue, &[]);
    let pairs = BroadphasePairs::collect(&accel);
    assert!(pairs.is_empty());
}

#[test]
fn single_collider_yields_no_pairs() {
    let Some((device, queue)) = skip_if_no_device() else { return; };
    let leaves = vec![collider_leaf(Vec3::ZERO, 0.5, 7)];
    let accel = pool_with_leaves(&device, &queue, &leaves);
    let pairs = BroadphasePairs::collect(&accel);
    assert!(pairs.is_empty());
}

#[test]
fn two_overlapping_colliders_yield_one_pair() {
    let Some((device, queue)) = skip_if_no_device() else { return; };
    let leaves = vec![
        collider_leaf(Vec3::ZERO, 0.5, 10),
        collider_leaf(Vec3::splat(0.3), 0.5, 20),
    ];
    let accel = pool_with_leaves(&device, &queue, &leaves);
    let pairs = BroadphasePairs::collect(&accel);
    assert_eq!(pairs.pairs(), &[(10, 20)]);
}

#[test]
fn two_disjoint_colliders_yield_no_pairs() {
    let Some((device, queue)) = skip_if_no_device() else { return; };
    let leaves = vec![
        collider_leaf(Vec3::ZERO, 0.5, 10),
        collider_leaf(Vec3::splat(10.0), 0.5, 20),
    ];
    let accel = pool_with_leaves(&device, &queue, &leaves);
    let pairs = BroadphasePairs::collect(&accel);
    assert!(pairs.is_empty());
}

#[test]
fn raymarch_only_leaves_are_ignored() {
    let Some((device, queue)) = skip_if_no_device() else { return; };
    // A raymarch-only leaf overlapping two colliders must NOT
    // produce raymarch↔collider pairs. Broadphase scopes itself
    // strictly to `IS_COLLIDER ↔ IS_COLLIDER` overlaps.
    let leaves = vec![
        collider_leaf(Vec3::ZERO, 0.5, 10),
        raymarch_only_leaf(Vec3::splat(0.2), 0.5, 99),
        collider_leaf(Vec3::splat(0.4), 0.5, 20),
    ];
    let accel = pool_with_leaves(&device, &queue, &leaves);
    let pairs = BroadphasePairs::collect(&accel);
    assert_eq!(pairs.pairs(), &[(10, 20)]);
}

#[test]
fn random_1000_colliders_match_brute_force() {
    let Some((device, queue)) = skip_if_no_device() else { return; };
    // 1000 colliders distributed in a 10×10×10 cube with radius 0.6
    // — enough overlap to exercise the BVH traversal pruning but not
    // so dense that brute force becomes meaningless.
    let mut rng_state = 0xC0DEC0DEu32;
    let mut rand = || {
        rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
        (rng_state >> 16) as f32 / 32768.0
    };
    let leaves: Vec<LeafAabb> = (0..1000u32)
        .map(|i| {
            let p = Vec3::new(rand(), rand(), rand()) * 10.0;
            collider_leaf(p, 0.6, i)
        })
        .collect();
    // 1000 leaves > AccelCaps::TEST.max_leaves (8192) — we have headroom
    // but pick a generous primitive_stride to stay aligned with the
    // existing test caps.
    let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
    let prim_bytes = vec![0u8; 16 * leaves.len()];
    accel
        .insert_chunk(
            &queue,
            ChunkInsert {
                key: 0,
                leaf_aabbs: &leaves,
                primitives_bytes: &prim_bytes,
                max_smoothness_radius: 0.0,
            },
        )
        .unwrap();
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);
    let pairs = BroadphasePairs::collect(&accel);

    let pool_set: HashSet<CollisionPair> = pairs.pairs().iter().copied().collect();
    let brute = brute_force_pairs(&leaves);
    assert_eq!(
        pool_set, brute,
        "broadphase pool pairs must match brute-force O(N²) ground truth",
    );
    assert_eq!(pool_set.len(), pairs.len(), "duplicate pair leaked through");
}

#[test]
fn dedup_canonicalises_low_high() {
    let Some((device, queue)) = skip_if_no_device() else { return; };
    let leaves = vec![
        collider_leaf(Vec3::ZERO, 0.5, 99),
        collider_leaf(Vec3::splat(0.3), 0.5, 5),
    ];
    let accel = pool_with_leaves(&device, &queue, &leaves);
    let pairs = BroadphasePairs::collect(&accel);
    assert_eq!(pairs.pairs(), &[(5, 99)]);
}

/// AC5 — cross-chunk collision detection.
///
/// Two chunks separated by ~6 m, each holding a collider whose AABB
/// straddles the chunk boundary. The TLAS descend visits both
/// chunks (their inflated descriptors overlap each collider's
/// query AABB), the per-chunk BLAS descend yields the matching
/// leaves, and the canonical `(low, high)` pair surfaces in
/// `BroadphasePairs::collect`.
///
/// Falsely missing this pair was the failure mode the issue body
/// pinned: a per-chunk broadphase that runs `O(C²)` *inside each
/// chunk* would never see the cross-chunk overlap. The pool-driven
/// path inherits the WGSL TLAS topology so the symmetric-query loop
/// handles cross-chunk implicitly.
#[test]
fn ac5_cross_chunk_overlap_detected_via_tlas() {
    let Some((device, queue)) = skip_if_no_device() else { return; };
    let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();

    // Chunk A: collider at x = -3, half-extent 4 → AABB x ∈ [-7, 1].
    // Chunk B: collider at x =  3, half-extent 4 → AABB x ∈ [-1, 7].
    // The two AABBs overlap in `[-1, 1]` — one cross-chunk pair.
    let leaves_a = vec![collider_leaf(Vec3::new(-3.0, 0.0, 0.0), 4.0, 100)];
    let leaves_b = vec![collider_leaf(Vec3::new( 3.0, 0.0, 0.0), 4.0, 200)];
    let prims_a = vec![0u8; 16];
    let prims_b = vec![0u8; 16];
    accel
        .insert_chunk(
            &queue,
            ChunkInsert {
                key: 1,
                leaf_aabbs: &leaves_a,
                primitives_bytes: &prims_a,
                max_smoothness_radius: 0.0,
            },
        )
        .unwrap();
    accel
        .insert_chunk(
            &queue,
            ChunkInsert {
                key: 2,
                leaf_aabbs: &leaves_b,
                primitives_bytes: &prims_b,
                max_smoothness_radius: 0.0,
            },
        )
        .unwrap();
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);

    let pairs = BroadphasePairs::collect(&accel);
    assert_eq!(
        pairs.pairs(),
        &[(100, 200)],
        "AC5: cross-chunk collider overlap must yield exactly one (100, 200) pair",
    );
}

/// Negative case for AC5: same setup but the colliders no longer
/// overlap (each fits inside its own chunk). The TLAS still descends
/// into both chunks because the chunk descriptors might overlap the
/// query AABB inflated by `max_smoothness_radius`, but the BLAS
/// leaves filter the candidate out — no spurious pair.
#[test]
fn ac5_disjoint_cross_chunk_yields_no_pair() {
    let Some((device, queue)) = skip_if_no_device() else { return; };
    let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();

    // Chunks 6 m apart, colliders half-extent 0.5 → no overlap.
    let leaves_a = vec![collider_leaf(Vec3::new(-3.0, 0.0, 0.0), 0.5, 100)];
    let leaves_b = vec![collider_leaf(Vec3::new( 3.0, 0.0, 0.0), 0.5, 200)];
    let prims_a = vec![0u8; 16];
    let prims_b = vec![0u8; 16];
    accel
        .insert_chunk(
            &queue,
            ChunkInsert {
                key: 1,
                leaf_aabbs: &leaves_a,
                primitives_bytes: &prims_a,
                max_smoothness_radius: 0.0,
            },
        )
        .unwrap();
    accel
        .insert_chunk(
            &queue,
            ChunkInsert {
                key: 2,
                leaf_aabbs: &leaves_b,
                primitives_bytes: &prims_b,
                max_smoothness_radius: 0.0,
            },
        )
        .unwrap();
    accel.update_gpu_standalone(&device, &queue, 0.0, 0.0);
    let pairs = BroadphasePairs::collect(&accel);
    assert!(pairs.is_empty(), "AC5: disjoint colliders must not pair");
}

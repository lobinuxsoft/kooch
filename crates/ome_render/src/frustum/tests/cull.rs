//! Frustum cull correctness tests. Compares the dispatched shader's
//! per-leaf `instance_count` output against the same plane-AABB
//! algorithm run on the CPU. Skipped when no GPU adapter is available.

use glam::Vec3;
use ome_bvh::{Aabb, IS_VISIBLE_MESH, LeafAabb, SharedBvhState};

use crate::frustum::cull::FrustumCull;

use super::harness::{
    axis_aligned_box_frustum, cpu_aabb_in_frustum, dispatch_and_readback,
    drive_build_to_completion, try_acquire_device, visible_mesh_scene,
};

#[test]
fn frustum_cull_all_inside_marks_every_leaf_visible() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("frustum_cull: no GPU adapter — skipping all-inside test");
        return;
    };
    // 8 cubes inside a unit cluster, frustum covers a wide box.
    let centres: Vec<Vec3> = (0..8).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect();
    let (items, leaves) = visible_mesh_scene(&centres, 0.4);
    let planes = axis_aligned_box_frustum(Vec3::splat(-100.0), Vec3::splat(100.0));

    let mut shared = SharedBvhState::new(&device, &queue, None);
    let _ = shared.kick(&device, &queue, items, leaves, /* hash */ 1);
    drive_build_to_completion(&mut shared, &device, &queue);

    let mut cull = FrustumCull::new(&device, None);
    let args = dispatch_and_readback(&device, &queue, &mut cull, &shared, &planes, 8);
    assert_eq!(args.len(), 8);
    for (i, a) in args.iter().enumerate() {
        assert_eq!(a.instance_count, 1, "leaf {i} should be visible");
        assert_eq!(a.first_instance, i as u32);
        assert_eq!(a.index_count, 36);
    }
}

#[test]
fn frustum_cull_all_outside_marks_every_leaf_culled() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("frustum_cull: no GPU adapter — skipping all-outside test");
        return;
    };
    // 8 cubes far outside a tiny frustum at origin.
    let centres: Vec<Vec3> = (0..8)
        .map(|i| Vec3::new(1000.0 + i as f32, 0.0, 0.0))
        .collect();
    let (items, leaves) = visible_mesh_scene(&centres, 0.4);
    let planes = axis_aligned_box_frustum(Vec3::splat(-1.0), Vec3::splat(1.0));

    let mut shared = SharedBvhState::new(&device, &queue, None);
    let _ = shared.kick(&device, &queue, items, leaves, /* hash */ 1);
    drive_build_to_completion(&mut shared, &device, &queue);

    let mut cull = FrustumCull::new(&device, None);
    let args = dispatch_and_readback(&device, &queue, &mut cull, &shared, &planes, 8);
    assert_eq!(args.len(), 8);
    for (i, a) in args.iter().enumerate() {
        assert_eq!(a.instance_count, 0, "leaf {i} should be culled");
    }
}

#[test]
fn frustum_cull_10k_cubes_matches_brute_force() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("frustum_cull: no GPU adapter — skipping 10k cubes test");
        return;
    };
    // 100x100 grid of unit cubes in the z=0 plane. Half-extent 0.4 so
    // the cubes are clearly separated; centres at integer coordinates.
    let mut centres = Vec::with_capacity(10_000);
    for j in 0..100 {
        for i in 0..100 {
            centres.push(Vec3::new(i as f32, j as f32, 0.0));
        }
    }
    let (items, leaves) = visible_mesh_scene(&centres, 0.4);
    let n = items.len() as u32;

    // Frustum: axis-aligned slice keeping x ≤ 49.5 and a fat y/z box.
    // The right plane bisects the grid between cube columns 49 and 50.
    let planes = axis_aligned_box_frustum(
        Vec3::new(-1000.0, -1000.0, -1000.0),
        Vec3::new(49.5, 1000.0, 1000.0),
    );

    let mut shared = SharedBvhState::new(&device, &queue, None);
    let _ = shared.kick(&device, &queue, items, leaves.clone(), /* hash */ 1);
    drive_build_to_completion(&mut shared, &device, &queue);

    let mut cull = FrustumCull::new(&device, None);
    let args = dispatch_and_readback(&device, &queue, &mut cull, &shared, &planes, n);
    assert_eq!(args.len(), n as usize);

    // CPU brute force using the exact same plane-AABB algorithm as the
    // shader. Result must agree byte-perfect — the GPU cull is just
    // the same computation parallelised.
    let mut mismatches = Vec::new();
    for (i, leaf) in leaves.iter().enumerate() {
        let cpu_visible = (leaf.flags & IS_VISIBLE_MESH != 0)
            && cpu_aabb_in_frustum(leaf.aabb_min, leaf.aabb_max, &planes.0);
        let gpu_visible = args[i].instance_count == 1;
        if cpu_visible != gpu_visible {
            mismatches.push(i);
        }
    }
    assert!(
        mismatches.is_empty(),
        "CPU/GPU disagreement on {} leaves (first 10: {:?})",
        mismatches.len(),
        &mismatches[..mismatches.len().min(10)],
    );

    // Belt-and-suspenders: total visible count equals the obvious
    // closed-form (cubes with center.x ∈ {0..49} pass the right plane).
    let visible_count = args.iter().filter(|a| a.instance_count == 1).count();
    let expected = 50 * 100; // x ∈ {0..49} × y ∈ {0..99}
    assert_eq!(
        visible_count, expected,
        "expected {expected} visible cubes for the x ≤ 49.5 slice",
    );
}

#[test]
fn frustum_cull_skips_non_mesh_leaves() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("frustum_cull: no GPU adapter — skipping non-mesh-skip test");
        return;
    };
    // Mixed scene: 4 IS_VISIBLE_MESH cubes + 4 with IS_VISIBLE_MESH
    // cleared. The clear-flagged ones must come back culled regardless
    // of where they sit relative to the frustum.
    let centres: Vec<Vec3> = (0..8).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect();
    let items: Vec<(u32, Aabb)> = centres
        .iter()
        .enumerate()
        .map(|(i, c)| (i as u32, Aabb::from_centre(*c, Vec3::splat(0.4))))
        .collect();
    let leaves: Vec<LeafAabb> = items
        .iter()
        .enumerate()
        .map(|(i, (id, a))| LeafAabb {
            aabb_min: a.min.to_array(),
            // Even indices are IS_VISIBLE_MESH; odd indices have the
            // bit cleared (e.g. raymarch-only or pure colliders).
            flags: if i % 2 == 0 { IS_VISIBLE_MESH } else { 0 },
            aabb_max: a.max.to_array(),
            entity_id: *id,
        })
        .collect();
    let planes = axis_aligned_box_frustum(Vec3::splat(-100.0), Vec3::splat(100.0));

    let mut shared = SharedBvhState::new(&device, &queue, None);
    let _ = shared.kick(&device, &queue, items, leaves, /* hash */ 1);
    drive_build_to_completion(&mut shared, &device, &queue);

    let mut cull = FrustumCull::new(&device, None);
    let args = dispatch_and_readback(&device, &queue, &mut cull, &shared, &planes, 8);
    for (i, a) in args.iter().enumerate() {
        let expect_visible = i % 2 == 0;
        assert_eq!(
            a.instance_count == 1,
            expect_visible,
            "leaf {i}: expected visible={expect_visible}, got instance_count={}",
            a.instance_count,
        );
    }
}

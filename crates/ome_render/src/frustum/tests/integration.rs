//! AC 116 multi-consumer integration test. Three consumers — raymarch
//! buffer accessors, [`ome_physics::BroadphasePairs`], and
//! [`super::super::cull::FrustumCull`] — all read from a single
//! [`SharedBvhState`]. The test builds a scene where leaves carry
//! mixed flags (IS_COLLIDER, IS_VISIBLE_MESH, IS_RAYMARCH) and verifies
//! each consumer scopes itself by its own bit.

use glam::Vec3;
use ome_bvh::{Aabb, IS_COLLIDER, IS_RAYMARCH, IS_VISIBLE_MESH, LeafAabb, SharedBvhState};

use crate::frustum::cull::FrustumCull;

use super::harness::{
    axis_aligned_box_frustum, dispatch_and_readback, drive_build_to_completion,
    try_acquire_device,
};

#[test]
fn ac116_three_consumers_share_one_bvh() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("frustum_cull: no GPU adapter — skipping AC 116 multi-consumer test");
        return;
    };

    // 12 leaves split into 3 cohorts × 4 leaves each. Each cohort
    // claims a different flag bit; collider+mesh entities overlap
    // intentionally so broadphase and frustum disagree on which
    // leaves matter (which is the whole point of separate flags).
    //
    // Layout per cohort: 4 cubes packed close enough that each pair
    // within the cohort overlaps. Cohort centres far apart so
    // broadphase never sees cross-cohort pairs.
    let mut centres = Vec::new();
    let mut flags = Vec::new();
    let cohort_offsets = [
        (Vec3::new(0.0, 0.0, 0.0), IS_COLLIDER),
        (Vec3::new(50.0, 0.0, 0.0), IS_VISIBLE_MESH),
        (Vec3::new(100.0, 0.0, 0.0), IS_RAYMARCH),
    ];
    for (cohort_origin, flag) in cohort_offsets {
        for k in 0..4 {
            // Within-cohort overlap: 0.5 spacing < 0.6 + 0.6 box reach.
            centres.push(cohort_origin + Vec3::new(k as f32 * 0.5, 0.0, 0.0));
            flags.push(flag);
        }
    }
    let half = 0.6;
    let items: Vec<(u32, Aabb)> = centres
        .iter()
        .enumerate()
        .map(|(i, c)| (i as u32, Aabb::from_centre(*c, Vec3::splat(half))))
        .collect();
    let leaves: Vec<LeafAabb> = items
        .iter()
        .enumerate()
        .map(|(i, (id, a))| LeafAabb {
            aabb_min: a.min.to_array(),
            flags: flags[i],
            aabb_max: a.max.to_array(),
            entity_id: *id,
        })
        .collect();
    let n = items.len() as u32;

    let mut shared = SharedBvhState::new(&device, &queue, None);
    let _ = shared.kick(&device, &queue, items.clone(), leaves.clone(), 1);
    drive_build_to_completion(&mut shared, &device, &queue);

    // --- Consumer #1: raymarch (would bind these buffers). ---
    // Verify the GPU buffers raymarch needs are populated.
    assert_eq!(shared.current_n(), n, "raymarch view: leaf count mismatch");
    let _nodes = shared.current_nodes();
    let _leaf_aabbs = shared.current_leaf_aabbs();
    let _sorted_indices = shared.current_sorted_indices();

    // --- Consumer #2: physics broadphase (CPU mirror). ---
    let pairs = ome_physics::BroadphasePairs::collect(&shared);
    // Cohort spacing 0.5, half 0.6: adjacent cubes overlap by 0.7,
    // two-step cubes overlap by 0.2, three-step cubes have a 0.3 gap.
    // 5 pairs — (0,1) (0,2) (1,2) (1,3) (2,3) — the (0,3) pair is
    // intentionally non-overlapping so the test also validates that
    // broadphase doesn't report spurious distant pairs.
    let expected: &[(u32, u32)] = &[(0, 1), (0, 2), (1, 2), (1, 3), (2, 3)];
    assert_eq!(
        pairs.pairs(),
        expected,
        "broadphase: pair set should match the only-overlapping-pairs ground truth",
    );
    for &(a, b) in pairs.pairs() {
        assert!(a < 4, "broadphase pair ({a},{b}): id {a} is not a collider");
        assert!(b < 4, "broadphase pair ({a},{b}): id {b} is not a collider");
    }

    // --- Consumer #3: frustum cull (GPU compute). ---
    // Frustum covers the entire scene; only IS_VISIBLE_MESH leaves
    // (cohort 2: ids 4..8) should come back visible.
    let planes = axis_aligned_box_frustum(Vec3::splat(-1000.0), Vec3::splat(1000.0));
    let mut cull = FrustumCull::new(&device, None);
    let args = dispatch_and_readback(&device, &queue, &mut cull, &shared, &planes, n);

    let mut visible_ids: Vec<u32> = Vec::new();
    for (i, a) in args.iter().enumerate() {
        if a.instance_count == 1 {
            visible_ids.push(i as u32);
        }
    }
    visible_ids.sort_unstable();
    assert_eq!(
        visible_ids,
        vec![4, 5, 6, 7],
        "frustum cull: expected only mesh-cohort ids visible, got {visible_ids:?}",
    );

    // --- Cross-consumer consistency. ---
    // CPU mirror leaf_aabbs and GPU buffer-bound leaf data both come
    // from the same kick — the orchestrator's job is to keep them
    // in sync; verify by checking the mirror reflects the same n.
    let mirror_leaves = shared
        .current_cpu_leaf_aabbs()
        .expect("cpu mirror present after build");
    assert_eq!(mirror_leaves.len(), n as usize);
    // Per-flag breakdown matches what each consumer saw.
    let collider_count = mirror_leaves
        .iter()
        .filter(|la| la.flags & IS_COLLIDER != 0)
        .count();
    let mesh_count = mirror_leaves
        .iter()
        .filter(|la| la.flags & IS_VISIBLE_MESH != 0)
        .count();
    let raymarch_count = mirror_leaves
        .iter()
        .filter(|la| la.flags & IS_RAYMARCH != 0)
        .count();
    assert_eq!(collider_count, 4);
    assert_eq!(mesh_count, 4);
    assert_eq!(raymarch_count, 4);
}

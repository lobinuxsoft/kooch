//! GPU integration tests for the GDF populate compute pass (PR-3 of
//! epic #370). Builds an `OmeAccel` from a known scene, runs the
//! populate dispatch through `GdfState`, reads the cascade texture
//! back, and asserts the per-voxel SDF matches a CPU mirror of
//! `eval_scene_bvh`. Skipped when no wgpu adapter is available —
//! same harness policy as `tests/pool_eval_smoke.rs`.
//!
//! Three core tests:
//! 1. `gdf_populate_matches_eval_scene_bvh_per_voxel` — single-sphere
//!    scene, sample 100 voxel centres, compare against the CPU
//!    mirror with Nyquist tolerance.
//! 2. `gdf_populate_empty_pool` — `live_chunk_count = 0` ⇒ every voxel
//!    holds `ACC_UNION_IDENTITY` (1.0e6).
//! 3. `gdf_populate_full_grid_no_zero_voxels` — 16-chunk procedural
//!    grid scene, no voxel exactly `0.0` (which would indicate a
//!    skipped storage write).
//!
//! Plus a CPU-only shape check for the cascade-origin shift in
//! `GdfState::dispatch_populate`.

mod common;

use common::gdf::{
    ACC_UNION_IDENTITY, build_16_chunk_accel, build_empty_accel, build_single_sphere_accel,
    build_two_sphere_accel, dispatch_and_readback, eval_scene_cpu, sdf_aabb_cpu, voxel_centre,
    voxel_index,
};
use common::try_acquire_device;
use glam::Vec3;
use ome_render::gdf::{
    CASCADE_0_VOXEL_SIZE, CASCADE_0_VOXELS_PER_AXIS, snap_to_voxel_grid,
};

const TOLERANCE_NYQUIST: f32 = CASCADE_0_VOXEL_SIZE * 0.5;
const TOLERANCE_BACKEND_NUMERIC: f32 = 1.0e-3;

/// Per-voxel quota — at least this many voxels of EACH category
/// (inside-AABB / smoothness band / far-from-surface) must be checked
/// before the test reports success. ≥34 × 3 = ≥102 total samples.
const PER_BUCKET_QUOTA: u32 = 34;

#[test]
fn gdf_populate_matches_eval_scene_bvh_per_voxel() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("skipping gdf_populate_matches — no adapter");
        return;
    };
    let (accel, prims, leaves, k_int, k_sub) = build_single_sphere_accel(&device, &queue);
    let (voxels, descriptor) = dispatch_and_readback(&device, &queue, &accel, Vec3::ZERO);

    // Single-sphere fixture: leaf AABB is inflated by `K_LEAF = 0.25`
    // (the per-leaf smoothness radius), so the voxel-classification
    // bands are derived from `sdf_aabb` against the leaf and the same
    // K_LEAF threshold the WGSL traversal uses for distance pruning.
    //
    // This test exercises the contract PR-4 will exploit via
    // `textureSampleLevel(gdf_cascade, p_world)`: the cascade carries
    // valid signed distances over ALL of R³, not only where the leaf
    // AABB overlaps the voxel centre. Pre-#381 the legacy
    // `aabb_contains(p)` pruning silenced every primitive whose AABB
    // did not contain the sample point, so the cascade outside any
    // leaf AABB was the `ACC_UNION_IDENTITY` sentinel — useless for a
    // far-from-surface single-fetch. Post-#381 the distance-to-AABB
    // pruning is conservative-equivalent to brute-force, so the CPU
    // mirror folds every primitive without an AABB gate (see
    // `eval_scene_cpu`).
    let leaf = &leaves[0];
    let leaf_lo = Vec3::from_array(leaf.aabb_min);
    let leaf_hi = Vec3::from_array(leaf.aabb_max);
    // K_LEAF used at construction time in `build_single_sphere_accel`.
    // Pin it at the same numeric value so the band classification
    // does not silently drift if the fixture changes.
    const SMOOTHNESS_RADIUS: f32 = 0.25;

    let n = CASCADE_0_VOXELS_PER_AXIS;
    let mut count_inside = 0u32;
    let mut count_band = 0u32;
    let mut count_far = 0u32;
    let mut max_diff_inside = 0.0f32;
    let mut max_diff_band = 0.0f32;
    let mut max_diff_far = 0.0f32;
    'outer: for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let centre = voxel_centre(&descriptor, x, y, z);
                let aabb_d = sdf_aabb_cpu(centre, leaf_lo, leaf_hi);
                let bucket = if aabb_d <= 0.0 {
                    if count_inside >= PER_BUCKET_QUOTA {
                        continue;
                    }
                    &mut count_inside
                } else if aabb_d <= SMOOTHNESS_RADIUS {
                    if count_band >= PER_BUCKET_QUOTA {
                        continue;
                    }
                    &mut count_band
                } else {
                    if count_far >= PER_BUCKET_QUOTA {
                        continue;
                    }
                    &mut count_far
                };

                let cpu = eval_scene_cpu(centre, &prims, &leaves, k_int, k_sub);
                let gpu = voxels[voxel_index(x, y, z)];
                let diff = (gpu - cpu).abs();
                let label = if aabb_d <= 0.0 {
                    max_diff_inside = max_diff_inside.max(diff);
                    "inside-AABB"
                } else if aabb_d <= SMOOTHNESS_RADIUS {
                    max_diff_band = max_diff_band.max(diff);
                    "smoothness-band"
                } else {
                    max_diff_far = max_diff_far.max(diff);
                    "far-from-surface"
                };
                assert!(
                    diff < TOLERANCE_NYQUIST,
                    "voxel ({x},{y},{z}) at {centre:?} ({label}, aabb_d={aabb_d:.4}): \
                     gpu={gpu} cpu={cpu} diff={diff}",
                );
                *bucket += 1;

                if count_inside >= PER_BUCKET_QUOTA
                    && count_band >= PER_BUCKET_QUOTA
                    && count_far >= PER_BUCKET_QUOTA
                {
                    break 'outer;
                }
            }
        }
    }
    assert!(
        count_inside >= PER_BUCKET_QUOTA
            && count_band >= PER_BUCKET_QUOTA
            && count_far >= PER_BUCKET_QUOTA,
        "per-bucket quota unmet: inside={count_inside} band={count_band} far={count_far} \
         (need ≥{PER_BUCKET_QUOTA} of each)"
    );
    eprintln!(
        "gdf_populate_matches: inside={count_inside} (max_diff {max_diff_inside:.6} m), \
         band={count_band} (max_diff {max_diff_band:.6} m), \
         far={count_far} (max_diff {max_diff_far:.6} m) — \
         Nyquist tolerance {TOLERANCE_NYQUIST} m"
    );
}

#[test]
fn gdf_populate_two_chunks_match_eval_scene_bvh() {
    // Closes #383: with a single-leaf TLAS the multi-leaf pruning
    // rule (`if sdf_aabb(p, leaf_far) > acc_add { continue; }`) is
    // dormant, so a regression that mishandles the bookkeeping
    // around `acc_add` carry-over between leaves can ride the
    // `gdf_populate_matches_eval_scene_bvh_per_voxel` test green.
    // Two non-overlapping spheres on the X axis force one leaf to
    // descend and the second to either descend (when its AABB is
    // closer than the running `acc_add`) or be pruned (when the
    // first leaf's union is already tighter). Both branches must
    // match brute-force.
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("skipping gdf_populate_two_chunks_match — no adapter");
        return;
    };
    const SEPARATION: f32 = 3.0;
    const RADIUS: f32 = 1.0;
    let (accel, prims, leaves, k_int, k_sub) =
        build_two_sphere_accel(&device, &queue, SEPARATION, RADIUS);
    let (voxels, descriptor) = dispatch_and_readback(&device, &queue, &accel, Vec3::ZERO);

    // Sample the line `x ∈ [-2 m, 2 m]`, `y = z = 0`. Voxel grid pitch
    // 0.25 m → 17 samples on the line, all at the y=z=midplane voxel
    // closest to the origin (cascade is centred on `(0, 0, 0)`, so
    // index = voxels_per_axis / 2). For each sample the running
    // `acc_add` after leaf 0 is the SDF of the `-SEPARATION` sphere;
    // leaf 1's AABB is closer to `+SEPARATION` so its `sdf_aabb` is
    // small near `x = +SEPARATION` (descend branch) and large near
    // `x = -SEPARATION` (prune branch). Crossing both branches in
    // one fixture is the point.
    let n = CASCADE_0_VOXELS_PER_AXIS;
    let mid = n / 2;
    let mut max_diff_descend = 0.0f32;
    let mut max_diff_prune = 0.0f32;
    let mut count_descend = 0u32;
    let mut count_prune = 0u32;
    for x in 0..n {
        let centre = voxel_centre(&descriptor, x, mid, mid);
        if centre.x.abs() > 2.5 {
            // Outside the [−2 m, 2 m] sample range.
            continue;
        }
        let cpu = eval_scene_cpu(centre, &prims, &leaves, k_int, k_sub);
        let gpu = voxels[voxel_index(x, mid, mid)];
        let diff = (gpu - cpu).abs();

        // Pruning branch is fired when leaf 1 (the +SEPARATION sphere)
        // would not improve the running union after leaf 0 descended.
        // That happens at voxels strictly closer to leaf 0 than to
        // leaf 1 — i.e. centre.x < 0 in this fixture.
        let leaf1 = &leaves[1];
        let leaf1_lo = Vec3::from_array(leaf1.aabb_min);
        let leaf1_hi = Vec3::from_array(leaf1.aabb_max);
        let aabb1 = sdf_aabb_cpu(centre, leaf1_lo, leaf1_hi);
        let leaf0 = &leaves[0];
        let leaf0_lo = Vec3::from_array(leaf0.aabb_min);
        let leaf0_hi = Vec3::from_array(leaf0.aabb_max);
        let aabb0 = sdf_aabb_cpu(centre, leaf0_lo, leaf0_hi);
        let label = if aabb1 < aabb0 { "descend" } else { "prune" };
        if aabb1 < aabb0 {
            max_diff_descend = max_diff_descend.max(diff);
            count_descend += 1;
        } else {
            max_diff_prune = max_diff_prune.max(diff);
            count_prune += 1;
        }

        assert!(
            diff < TOLERANCE_NYQUIST,
            "voxel ({x},{mid},{mid}) at {centre:?} ({label}): \
             gpu={gpu} cpu={cpu} diff={diff} > {TOLERANCE_NYQUIST}"
        );
    }
    assert!(
        count_descend > 0 && count_prune > 0,
        "fixture failed to exercise both BVH branches: descend={count_descend} prune={count_prune}"
    );
    eprintln!(
        "gdf_populate_two_chunks_match: descend={count_descend} (max_diff {max_diff_descend:.6} m), \
         prune={count_prune} (max_diff {max_diff_prune:.6} m) — Nyquist tolerance {TOLERANCE_NYQUIST} m"
    );
}

#[test]
fn gdf_populate_empty_pool() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("skipping gdf_populate_empty_pool — no adapter");
        return;
    };
    let accel = build_empty_accel(&device, &queue);
    let (voxels, _descriptor) = dispatch_and_readback(&device, &queue, &accel, Vec3::ZERO);
    let n_total = (CASCADE_0_VOXELS_PER_AXIS as usize).pow(3);
    assert_eq!(voxels.len(), n_total);
    for (i, v) in voxels.iter().enumerate() {
        assert!(
            (*v - ACC_UNION_IDENTITY).abs() < TOLERANCE_BACKEND_NUMERIC,
            "voxel {i}: expected ACC_UNION_IDENTITY ({ACC_UNION_IDENTITY}), got {v}"
        );
    }
}

#[test]
fn gdf_populate_full_grid_no_zero_voxels() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("skipping gdf_populate_full_grid_no_zero_voxels — no adapter");
        return;
    };
    let accel = build_16_chunk_accel(&device, &queue);
    let (voxels, _descriptor) = dispatch_and_readback(&device, &queue, &accel, Vec3::ZERO);

    // No voxel should end up *exactly* 0.0 — that's the sentinel for
    // "compute thread early-returned without writing", which would
    // mean the populate dispatch grid is mis-sized. A genuine
    // surface crossing at exactly 0.0 has ULP-zero probability with
    // f32 voxel centres.
    let zeros: Vec<usize> = voxels
        .iter()
        .enumerate()
        .filter_map(|(i, v)| (*v == 0.0).then_some(i))
        .collect();
    assert!(
        zeros.is_empty(),
        "{} voxels are exactly 0.0 — write skip detected (first 8: {:?})",
        zeros.len(),
        &zeros[..zeros.len().min(8)]
    );
}

#[test]
fn cascade_descriptor_origin_centres_around_camera() {
    // Smoke test for the cascade-origin shift in
    // `GdfState::dispatch_populate`. Camera at (4.2, -1.7, 7.7) with
    // 16 m cascade ⇒ origin = snap(camera) - half_extent. The camera
    // must land inside the cascade AABB for the populate to be useful.
    let camera = Vec3::new(4.2, -1.7, 7.7);
    let snapped = snap_to_voxel_grid(camera, CASCADE_0_VOXEL_SIZE);
    let half_extent =
        CASCADE_0_VOXEL_SIZE * (CASCADE_0_VOXELS_PER_AXIS as f32 * 0.5);
    let origin = snapped - Vec3::splat(half_extent);
    let max = origin + Vec3::splat(2.0 * half_extent);
    assert!(
        camera.x >= origin.x && camera.y >= origin.y && camera.z >= origin.z,
        "camera below cascade origin: camera={camera:?} origin={origin:?}"
    );
    assert!(
        camera.x <= max.x && camera.y <= max.y && camera.z <= max.z,
        "camera above cascade max: camera={camera:?} max={max:?}"
    );
}

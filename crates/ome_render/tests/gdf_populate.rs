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
    dispatch_and_readback, eval_scene_cpu, voxel_centre, voxel_index,
};
use common::try_acquire_device;
use glam::Vec3;
use ome_render::gdf::{
    CASCADE_0_VOXEL_SIZE, CASCADE_0_VOXELS_PER_AXIS, snap_to_voxel_grid,
};

const TOLERANCE_NYQUIST: f32 = CASCADE_0_VOXEL_SIZE * 0.5;
const TOLERANCE_BACKEND_NUMERIC: f32 = 1.0e-3;

#[test]
fn gdf_populate_matches_eval_scene_bvh_per_voxel() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("skipping gdf_populate_matches — no adapter");
        return;
    };
    let (accel, prims, leaves, k_int, k_sub) = build_single_sphere_accel(&device, &queue);
    let (voxels, descriptor) = dispatch_and_readback(&device, &queue, &accel, Vec3::ZERO);

    // Sample voxels that overlap the inflated sphere AABB — those are
    // the only ones with a non-trivial SDF. Outside-AABB voxels match
    // `ACC_UNION_IDENTITY` regardless of fold order, so they don't
    // exercise the per-voxel evaluation we want to validate.
    let n = CASCADE_0_VOXELS_PER_AXIS;
    let leaf = &leaves[0];
    let mut checked = 0u32;
    let mut max_diff = 0.0f32;
    'outer: for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let centre = voxel_centre(&descriptor, x, y, z);
                let inside_aabb = (centre.x >= leaf.aabb_min[0])
                    && (centre.y >= leaf.aabb_min[1])
                    && (centre.z >= leaf.aabb_min[2])
                    && (centre.x <= leaf.aabb_max[0])
                    && (centre.y <= leaf.aabb_max[1])
                    && (centre.z <= leaf.aabb_max[2]);
                if !inside_aabb {
                    continue;
                }
                let cpu = eval_scene_cpu(centre, &prims, &leaves, k_int, k_sub);
                let gpu = voxels[voxel_index(x, y, z)];
                let diff = (gpu - cpu).abs();
                max_diff = max_diff.max(diff);
                assert!(
                    diff < TOLERANCE_NYQUIST,
                    "voxel ({x},{y},{z}) at {centre:?}: gpu={gpu} cpu={cpu} diff={diff}",
                );
                checked += 1;
                if checked >= 100 {
                    break 'outer;
                }
            }
        }
    }
    assert!(
        checked >= 100,
        "expected ≥100 inside-AABB voxels, got {checked}"
    );
    eprintln!(
        "gdf_populate_matches: checked {checked} voxels, max_diff = {max_diff:.6} m \
         (Nyquist tolerance {TOLERANCE_NYQUIST} m, observed within numeric noise)"
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

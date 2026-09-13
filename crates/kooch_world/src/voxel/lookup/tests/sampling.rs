//! GPU end-to-end sampling tests — drive `classify → populate → probe`
//! through [`super::harness::run_lookup_probes`] and assert the lookup
//! semantics on the readback. Skip cleanly when no GPU is available.

use super::harness::{
    cell_min_world, run_lookup_probes, run_lookup_probes_with_target, test_bounds,
};
use crate::voxel::{
    ALLOC_FAILED_SENTINEL, AnalyticSphereSampler, LOD_LEVELS, ROOT_CELLS, ROOT_DIM, SUBGRID_DIM,
    SUBGRID_VOXELS, test_device,
};
use glam::Vec3;

/// Smaller with `large-root-grid`, whose 32³ grid meets ~13× more cells, keeping allocation under
/// 1024 slots and indices deterministic.
#[cfg(not(feature = "large-root-grid"))]
const TEST_SPHERE_RADIUS: f32 = 16.0;
#[cfg(feature = "large-root-grid")]
const TEST_SPHERE_RADIUS: f32 = 8.0;

/// f16's quantum scales with the value, so tolerance is `1e-3 × |expected|` with a `1e-3` floor for
/// values near the surface.
fn f16_lookup_tolerance(expected: f32) -> f32 {
    (expected.abs() * 1.0e-3).max(1.0e-3)
}

#[test]
fn lookup_at_voxel_corners_returns_pool_values() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping lookup_at_voxel_corners_returns_pool_values: no GPU");
        return;
    };
    // First run: no probes — used only to discover which cells got
    // allocated, then we craft the corner probes.
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), TEST_SPHERE_RADIUS);
    let bounds = test_bounds();
    // Probe at one position (origin of bounds); we will read
    // root_indices to pick allocated cells, then re-run with the
    // crafted positions.
    let bootstrap = run_lookup_probes(&device, &queue, &sampler, bounds, 1024, &[Vec3::splat(0.0)]);

    // Pick up to 10 allocated cells deterministically (lowest cell
    // indices first → reproducible across runs).
    let allocated_cells: Vec<u32> = (0..ROOT_CELLS)
        .filter(|&idx| bootstrap.root_indices[idx as usize] < 1024)
        .take(10)
        .collect();
    assert!(
        !allocated_cells.is_empty(),
        "test scene mis-tuned: expected at least one allocated cell",
    );

    let extent = bounds.max - bounds.min;
    let cell_size = extent / (ROOT_DIM as f32);
    // 50 voxels per cell from a cheap LCG — reproducible without `rand`, skipping corners the
    // shader clamps.
    let mut probe_positions: Vec<Vec3> = Vec::new();
    let mut probe_meta: Vec<(usize, u32)> = Vec::new();
    for &cell_idx in &allocated_cells {
        let cell_min = cell_min_world(cell_idx, bounds);
        let mut state: u32 = cell_idx.wrapping_mul(0x9E3779B1).wrapping_add(1);
        for _ in 0..50 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let voxel_linear = state % SUBGRID_VOXELS;
            let vz = voxel_linear / (SUBGRID_DIM * SUBGRID_DIM);
            let vy = (voxel_linear / SUBGRID_DIM) % SUBGRID_DIM;
            let vx = voxel_linear % SUBGRID_DIM;
            let voxel_offset = Vec3::new(vx as f32, vy as f32, vz as f32) / (SUBGRID_DIM as f32);
            probe_positions.push(cell_min + voxel_offset * cell_size);
            probe_meta.push((cell_idx as usize, voxel_linear));
        }
    }

    let run = run_lookup_probes(&device, &queue, &sampler, bounds, 1024, &probe_positions);
    // At a voxel corner trilinear collapses to one texel, so the lookup equals the analytic sampler
    // within f16 quantisation.
    for (i, &(cell_idx, _voxel_linear)) in probe_meta.iter().enumerate() {
        let subgrid_idx = run.root_indices[cell_idx];
        assert!(subgrid_idx < 1024, "cell {cell_idx} should be allocated");
        let expected = sampler.sample_cpu(probe_positions[i]);
        let actual = run.results[i];
        let tol = f16_lookup_tolerance(expected);
        assert!(
            (actual - expected).abs() < tol,
            "probe {i} cell {cell_idx}: GPU lookup {actual} vs CPU sampler {expected} \
             (tolerance {tol})",
        );
    }
}

#[test]
fn lookup_trilinear_midpoint_matches_corner_average() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping lookup_trilinear_midpoint_matches_corner_average: no GPU");
        return;
    };
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), TEST_SPHERE_RADIUS);
    let bounds = test_bounds();
    // Bootstrap run: discover allocated cells.
    let bootstrap = run_lookup_probes(&device, &queue, &sampler, bounds, 1024, &[Vec3::splat(0.0)]);
    let cell_idx = (0..ROOT_CELLS)
        .find(|&idx| bootstrap.root_indices[idx as usize] < 1024)
        .expect("expected at least one allocated cell");

    let extent = bounds.max - bounds.min;
    let cell_size = extent / (ROOT_DIM as f32);
    let voxel_size = cell_size / (SUBGRID_DIM as f32);
    let cell_min = cell_min_world(cell_idx, bounds);

    // Sample at the midpoint between voxel(0,0,0) and voxel(1,0,0) of
    // this cell — `f.x = 0.5`, `f.y = f.z = 0`. Trilinear collapses
    // to `0.5 * (s_v000 + s_v100)`.
    let v000_world = cell_min;
    let midpoint = cell_min + Vec3::new(0.5, 0.0, 0.0) * voxel_size;
    let v100_world = cell_min + Vec3::new(1.0, 0.0, 0.0) * voxel_size;

    let run = run_lookup_probes(
        &device,
        &queue,
        &sampler,
        bounds,
        1024,
        &[v000_world, v100_world, midpoint],
    );
    let s000 = run.results[0];
    let s100 = run.results[1];
    let mid = run.results[2];
    let expected = 0.5 * (s000 + s100);
    let tol = f16_lookup_tolerance(expected);
    assert!(
        (mid - expected).abs() < tol,
        "trilinear midpoint {mid} vs corner average {expected} (tolerance {tol})",
    );
}

#[test]
fn lookup_in_empty_cell_returns_far_sentinel() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping lookup_in_empty_cell_returns_far_sentinel: no GPU");
        return;
    };
    // A small sphere at the low corner leaves cell (15, 15, 15) empty, so a lookup at its centre
    // returns `2 * cell_size = 8.0`.
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(8.0), 4.0);
    let bounds = test_bounds();
    let probe = Vec3::splat(56.0);
    let run = run_lookup_probes(&device, &queue, &sampler, bounds, 256, &[probe]);

    // Sanity: the cell containing `probe` is actually empty.
    let cell = ((probe - bounds.min) / ((bounds.max - bounds.min) / ROOT_DIM as f32)).floor();
    let cell_idx =
        (cell.x as u32) + (cell.y as u32) * ROOT_DIM + (cell.z as u32) * ROOT_DIM * ROOT_DIM;
    assert_eq!(
        run.root_indices[cell_idx as usize], 0xFFFFFFFFu32,
        "test scene mis-tuned: probe cell should be empty",
    );

    let cell_size = (bounds.max - bounds.min) / ROOT_DIM as f32;
    let expected = cell_size.x.max(cell_size.y).max(cell_size.z) * 2.0;
    assert_eq!(run.results[0], expected);
}

#[test]
fn lookup_out_of_bounds_returns_far_sentinel() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping lookup_out_of_bounds_returns_far_sentinel: no GPU");
        return;
    };
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), TEST_SPHERE_RADIUS);
    let bounds = test_bounds();
    let below = bounds.min - Vec3::splat(1.0);
    let above = bounds.max + Vec3::splat(1.0);
    let run = run_lookup_probes(&device, &queue, &sampler, bounds, 256, &[below, above]);
    let cell_size = (bounds.max - bounds.min) / ROOT_DIM as f32;
    let expected = cell_size.x.max(cell_size.y).max(cell_size.z) * 2.0;
    assert_eq!(run.results[0], expected, "below bounds_min must return far");
    assert_eq!(run.results[1], expected, "above bounds_max must return far");
}

#[test]
fn lookup_with_target_voxel_size_selects_correct_lod() {
    // The same point at LOD 0 (0.25 pitch) and LOD 2 (1.0) must both agree with the surface within
    // each LOD's quantisation.
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping lookup_with_target_voxel_size_selects_correct_lod: no GPU");
        return;
    };
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), TEST_SPHERE_RADIUS);
    let bounds = test_bounds();
    // Just inside the surface: 8 units from centre, 4 with the smaller `large-root-grid` sphere.
    #[cfg(not(feature = "large-root-grid"))]
    let probe = Vec3::splat(24.0);
    #[cfg(feature = "large-root-grid")]
    let probe = Vec3::splat(28.0);
    // 64.0 / ROOT_DIM → 4.0 (default) or 2.0 (large-root-grid).
    let cell_size = 64.0 / (ROOT_DIM as f32);
    let voxel_pitch_lod0 = cell_size / (LOD_LEVELS[0].subgrid_dim as f32);
    let voxel_pitch_lod2 = voxel_pitch_lod0 * LOD_LEVELS[2].voxel_size_factor;

    let run_lod0 = run_lookup_probes_with_target(
        &device,
        &queue,
        &sampler,
        bounds,
        1024,
        &[probe],
        voxel_pitch_lod0,
    );
    let run_lod2 = run_lookup_probes_with_target(
        &device,
        &queue,
        &sampler,
        bounds,
        1024,
        &[probe],
        voxel_pitch_lod2,
    );

    let cpu = sampler.sample_cpu(probe);
    // LOD 0 tolerance: f16 quantum × value (≈ 1e-3 relative).
    let tol_lod0 = (cpu.abs() * 1.0e-3).max(1.0e-3);
    assert!(
        (run_lod0.results[0] - cpu).abs() < tol_lod0,
        "LOD 0 lookup {} vs CPU {} (tol {tol_lod0})",
        run_lod0.results[0],
        cpu,
    );
    // LOD 2 is box-filtered 4× — tolerance scales with the LOD's
    // voxel pitch (the discretisation error of a smooth SDF over a
    // 4× coarser grid is bounded by the voxel pitch itself).
    let tol_lod2 = voxel_pitch_lod2 * 2.0;
    assert!(
        (run_lod2.results[0] - cpu).abs() < tol_lod2,
        "LOD 2 lookup {} vs CPU {} (tol {tol_lod2})",
        run_lod2.results[0],
        cpu,
    );
    // Distinct LODs may yield slightly different values; both must
    // have the same sign — the surface side is LOD-invariant.
    assert!(
        run_lod0.results[0].is_sign_positive() == run_lod2.results[0].is_sign_positive()
            || run_lod0.results[0].abs() < tol_lod0
            || run_lod2.results[0].abs() < tol_lod2,
        "LOD 0 ({}) and LOD 2 ({}) must agree on surface side away from zero",
        run_lod0.results[0],
        run_lod2.results[0],
    );
}

#[test]
fn lookup_in_alloc_failed_cell_returns_far_sentinel() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping lookup_in_alloc_failed_cell_returns_far_sentinel: no GPU");
        return;
    };
    // Pool capacity 4 with the 64³ sphere/16 scene → ~780 cells
    // marked, only 4 succeed. Find one that landed on
    // ALLOC_FAILED_SENTINEL and probe its centre.
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), TEST_SPHERE_RADIUS);
    let bounds = test_bounds();
    let bootstrap = run_lookup_probes(&device, &queue, &sampler, bounds, 4, &[Vec3::splat(0.0)]);
    let failed_cell = (0..ROOT_CELLS)
        .find(|&idx| bootstrap.root_indices[idx as usize] == ALLOC_FAILED_SENTINEL)
        .expect("expected at least one ALLOC_FAILED cell with capacity 4");
    let cell_size = (bounds.max - bounds.min) / ROOT_DIM as f32;
    let cz = failed_cell / (ROOT_DIM * ROOT_DIM);
    let cy = (failed_cell / ROOT_DIM) % ROOT_DIM;
    let cx = failed_cell % ROOT_DIM;
    let probe =
        bounds.min + Vec3::new(cx as f32, cy as f32, cz as f32) * cell_size + cell_size * 0.5;

    let run = run_lookup_probes(&device, &queue, &sampler, bounds, 4, &[probe]);
    let expected = cell_size.x.max(cell_size.y).max(cell_size.z) * 2.0;
    assert_eq!(run.results[0], expected);
    // Discard `grid` ownership warning — read indirectly via the run.
    let _ = run.grid;
}

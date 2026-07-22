//! Tests for [`crate::voxel::metrics`]. CPU/naga checks for the
//! shader + struct sizing, plus a GPU end-to-end that runs the full
//! cascade and verifies post-cascade metrics match the canonical
//! invariants enforced by the b18f4aa fix-up:
//!
//! - `active_subgrids[0] == needs_count_lod0` (LOD 0 is the only
//!   producer).
//! - `active_subgrids[i > 0] == 0` (downsample copies idx, never pops
//!   the higher-LOD freelists).
//! - `alloc_count_total == active_subgrids[0]` (one pop per marked
//!   cell at LOD 0; no frees in the canonical chain).
//! - `free_count_total == 0`.
//! - `vram_bytes` matches the constexpr [`LOD_LEVELS`] sum.

use super::{METRICS_WGSL, Metrics, MetricsPass};
use crate::voxel::{
    AnalyticSphereSampler, CASCADE_COUNT, ClassifyPass, DEFAULT_LOD_DISTANCE_THRESHOLDS,
    DEFAULT_MARGIN, DownsamplePass, LOD_COUNT, LOD_LEVELS, METRICS_BUFFER_SIZE,
    PopulatePass, SdfSampler, SparseGrid, test_device, test_device::readback,
};
use glam::Vec3;
use ome_core::Aabb;

/// Sphere radius the metrics cascade test exercises. Mirrors the
/// scaling done in `populate/tests.rs` — see the comment there.
#[cfg(not(feature = "large-root-grid"))]
const TEST_SPHERE_RADIUS: f32 = 16.0;
#[cfg(feature = "large-root-grid")]
const TEST_SPHERE_RADIUS: f32 = 8.0;

#[test]
fn metrics_wgsl_parses_and_validates() {
    let module = naga::front::wgsl::parse_str(METRICS_WGSL)
        .expect("sparse_metrics.wgsl should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("sparse_metrics.wgsl should validate");
}

#[test]
fn metrics_buffer_size_constexpr() {
    // 4 LODs × 4 B + 4 B alloc + 4 B free = 24 B.
    assert_eq!(METRICS_BUFFER_SIZE, 24);
    assert_eq!(METRICS_BUFFER_SIZE, ((LOD_COUNT as u64) + 2) * 4);
}

#[test]
fn metrics_vram_bytes_matches_lod_table() {
    let expected: u64 = LOD_LEVELS
        .iter()
        .map(|lod| {
            (lod.atlas_dim_x as u64)
                * (lod.atlas_dim_y as u64)
                * (lod.atlas_dim_z as u64)
                * 2
        })
        .sum();
    assert_eq!(Metrics::vram_bytes_from_lod_table(), expected);
    // AC1 (#136) caps default at 15 MiB / chunk; AC4 (#347) caps the
    // `large-root-grid` build at 100 MiB / chunk. Floor stays the
    // LOD-0 atlas size so a partial allocation regression trips here.
    let cap_bytes: u64 = if cfg!(feature = "large-root-grid") {
        100 * 1024 * 1024
    } else {
        15 * 1024 * 1024
    };
    let floor_bytes: u64 = if cfg!(feature = "large-root-grid") {
        19 * 1024 * 1024
    } else {
        11 * 1024 * 1024
    };
    assert!(expected < cap_bytes, "AC: < {cap_bytes} bytes / chunk");
    assert!(expected > floor_bytes, "sanity: 4 atlases present");
}

fn run_cascade_and_read_metrics(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bounds: Aabb,
    radius: f32,
) -> (Metrics, u32) {
    let max_subgrids = LOD_LEVELS[0].max_subgrids;
    let grid = SparseGrid::new(device, queue, bounds, max_subgrids);

    // Force every LOD active so the cascade runs end-to-end. Skipping
    // the chunk_lod pass keeps the test deterministic w.r.t. the
    // active_origin sentinel; the harness pattern from lookup tests.
    let all = (1u32 << LOD_COUNT) - 1;
    queue.write_buffer(grid.chunk_lod_mask_buffer(), 0, &all.to_le_bytes());

    let center = (bounds.min + bounds.max) * 0.5;
    let sampler = AnalyticSphereSampler::new(device, center, radius);
    let classify = ClassifyPass::new(
        device,
        sampler.wgsl_source(),
        &sampler.bind_group_layout_entries(),
    );
    let populate = PopulatePass::new(
        device,
        sampler.wgsl_source(),
        &sampler.bind_group_layout_entries(),
    );
    let downsample = DownsamplePass::new(device);
    let metrics_pass = MetricsPass::new(device);

    let sampler_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::metrics::sampler_bg"),
        layout: classify.sampler_bind_group_layout(),
        entries: &sampler.bind_group_entries(),
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test::metrics::encoder"),
    });
    classify.record(
        device, queue, &mut encoder, &grid, &sampler_bg, 0, DEFAULT_MARGIN,
    );
    populate.record_finalize(device, &mut encoder, &grid, 0);
    populate.record_populate(
        device, queue, &mut encoder, &grid, &sampler_bg, 0,
    );
    for cascade_idx in 0..(CASCADE_COUNT as u32) {
        downsample.record_cascade(device, &mut encoder, &grid, cascade_idx);
    }
    metrics_pass.record(device, &mut encoder, &grid);
    queue.submit(std::iter::once(encoder.finish()));

    let needs_count_bytes = readback(device, queue, grid.needs_count_buffer(0));
    let needs_count = u32::from_le_bytes([
        needs_count_bytes[0],
        needs_count_bytes[1],
        needs_count_bytes[2],
        needs_count_bytes[3],
    ]);

    let metrics = Metrics::read(&grid, device, queue);
    (metrics, needs_count)
}

#[test]
fn metrics_post_cascade_matches_canonical_invariants() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping metrics_post_cascade_matches_canonical_invariants: no GPU");
        return;
    };
    let bounds = Aabb::new(Vec3::ZERO, Vec3::splat(64.0));
    let (metrics, needs_count) =
        run_cascade_and_read_metrics(&device, &queue, bounds, TEST_SPHERE_RADIUS);

    assert!(
        needs_count > 0,
        "test setup: sphere should mark at least one root cell at LOD 0",
    );
    assert_eq!(
        metrics.active_subgrids[0],
        needs_count,
        "LOD 0: one freelist pop per marked cell",
    );
    for lod in 1..LOD_COUNT as usize {
        assert_eq!(
            metrics.active_subgrids[lod],
            0,
            "LOD {lod}: downsample copies idx, never pops higher-LOD freelist",
        );
    }
    assert_eq!(
        metrics.alloc_count_total,
        needs_count,
        "alloc_count_total must equal LOD 0 pops (no pops at higher LODs)",
    );
    assert_eq!(
        metrics.free_count_total,
        0,
        "canonical cascade never frees — push counter must stay 0",
    );
    assert_eq!(
        metrics.vram_bytes,
        Metrics::vram_bytes_from_lod_table(),
        "vram_bytes is host-derived from constexpr LOD_LEVELS",
    );
}

#[test]
fn metrics_zero_cascade_writes_zero_active() {
    // Build a fresh grid + dispatch metrics without running the cascade.
    // Every freelist still has free_top = max_subgrids → every active = 0.
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping metrics_zero_cascade_writes_zero_active: no GPU");
        return;
    };
    let bounds = Aabb::new(Vec3::ZERO, Vec3::splat(64.0));
    let grid = SparseGrid::new(&device, &queue, bounds, LOD_LEVELS[0].max_subgrids);
    let metrics_pass = MetricsPass::new(&device);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test::metrics::zero_encoder"),
    });
    metrics_pass.record(&device, &mut encoder, &grid);
    queue.submit(std::iter::once(encoder.finish()));

    let metrics = Metrics::read(&grid, &device, &queue);
    assert_eq!(metrics.active_subgrids, [0; LOD_COUNT as usize]);
    assert_eq!(metrics.alloc_count_total, 0);
    assert_eq!(metrics.free_count_total, 0);
}

/// Sanity: orchestrator-recorded cascade ends with metrics populated
/// (covers the wiring at `SparseLodPass::record`).
#[test]
fn orchestrator_records_metrics_pass() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping orchestrator_records_metrics_pass: no GPU");
        return;
    };
    let bounds = Aabb::new(Vec3::ZERO, Vec3::splat(64.0));
    let center = (bounds.min + bounds.max) * 0.5;
    let sampler = AnalyticSphereSampler::new(&device, center, TEST_SPHERE_RADIUS);
    let lod_pass = crate::voxel::SparseLodPass::new(
        &device,
        sampler.wgsl_source(),
        &sampler.bind_group_layout_entries(),
    );
    let grid = SparseGrid::new(&device, &queue, bounds, LOD_LEVELS[0].max_subgrids);
    let sampler_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::metrics::orchestrator_sampler_bg"),
        layout: lod_pass.sampler_bind_group_layout(),
        entries: &sampler.bind_group_entries(),
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test::metrics::orchestrator_encoder"),
    });
    lod_pass.record(
        &device,
        &queue,
        &mut encoder,
        &grid,
        &sampler_bg,
        center,
        DEFAULT_LOD_DISTANCE_THRESHOLDS,
        DEFAULT_MARGIN,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let metrics = Metrics::read(&grid, &device, &queue);
    assert!(
        metrics.active_subgrids[0] > 0,
        "orchestrator should populate at least one LOD 0 subgrid",
    );
    assert_eq!(
        metrics.alloc_count_total, metrics.active_subgrids[0],
        "alloc total = LOD 0 pops in canonical orchestrator chain",
    );
    assert_eq!(metrics.free_count_total, 0);
}

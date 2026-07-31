//! Tests for [`crate::voxel::classify`] — kept in their own file so
//! the impl module stays under the no-monolithic threshold. The GPU
//! tests share helpers ([`run_classify`], [`cpu_classify`]) so each
//! test stays focused on the assertion it makes.
//!
//! Every GPU test gates on [`test_device::try_acquire`] and skips
//! cleanly when no adapter is available — CI without a display, or a
//! sandbox without GPU passthrough, is expected.

use super::{CLASSIFY_WGSL, ClassifyPass, DEFAULT_MARGIN};
use crate::voxel::{
    ANALYTIC_SPHERE_WGSL, AnalyticSphereSampler, LOD_COUNT, ROOT_CELLS, ROOT_DIM,
    SdfSampler, SparseGrid, test_device,
};
use glam::Vec3;
use kooch_core::Aabb;
use std::collections::HashSet;

const TEST_BOUNDS_MIN: Vec3 = Vec3::ZERO;
const TEST_BOUNDS_MAX: Vec3 = Vec3::splat(64.0);

fn test_bounds() -> Aabb {
    Aabb::new(TEST_BOUNDS_MIN, TEST_BOUNDS_MAX)
}

/// Force every LOD bit on in the chunk-LOD mask so per-LOD classify
/// pipelines do not early-out. Tests that want to exercise the mask
/// gating explicitly write a different value.
fn enable_all_lods(queue: &wgpu::Queue, grid: &SparseGrid) {
    let all = (1u32 << LOD_COUNT) - 1;
    queue.write_buffer(grid.chunk_lod_mask_buffer(), 0, &all.to_le_bytes());
}

/// CPU mirror of the WGSL Lipschitz cone test in `sparse_classify.wgsl`.
fn cpu_classify(sampler: &AnalyticSphereSampler, bounds: Aabb, margin: f32) -> Vec<u32> {
    let extent = bounds.max - bounds.min;
    let cell_size = extent / (ROOT_DIM as f32);
    let cell_diag = cell_size.length();
    let mut marks = Vec::new();
    for cell_idx in 0..ROOT_CELLS {
        let cz = cell_idx / (ROOT_DIM * ROOT_DIM);
        let cy = (cell_idx / ROOT_DIM) % ROOT_DIM;
        let cx = cell_idx % ROOT_DIM;
        let cell_3d = Vec3::new(cx as f32, cy as f32, cz as f32);
        let center = bounds.min + (cell_3d + Vec3::splat(0.5)) * cell_size;
        let sdf = sampler.sample_cpu(center);
        if sdf.abs() < cell_diag * margin {
            marks.push(cell_idx);
        }
    }
    marks
}

/// Run one `ClassifyPass::record` against `sampler` at `lod_idx`.
fn run_classify_at_lod(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sampler: &AnalyticSphereSampler,
    margin: f32,
    lod_idx: u32,
) -> (u32, Vec<u32>) {
    let grid = SparseGrid::new(device, queue, test_bounds(), 256);
    enable_all_lods(queue, &grid);
    let pass = ClassifyPass::new(
        device,
        sampler.wgsl_source(),
        &sampler.bind_group_layout_entries(),
    );
    let sampler_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::sampler_bg"),
        layout: pass.sampler_bind_group_layout(),
        entries: &sampler.bind_group_entries(),
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test::classify_encoder"),
    });
    pass.record(device, queue, &mut encoder, &grid, &sampler_bg, lod_idx, margin);
    queue.submit(std::iter::once(encoder.finish()));

    let count_bytes =
        test_device::readback(device, queue, grid.needs_count_buffer(lod_idx));
    let count = u32::from_le_bytes([
        count_bytes[0],
        count_bytes[1],
        count_bytes[2],
        count_bytes[3],
    ]);

    let indices_bytes =
        test_device::readback(device, queue, grid.needs_indices_buffer(lod_idx));
    let mut indices: Vec<u32> = indices_bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    indices.truncate(count as usize);

    (count, indices)
}

#[test]
fn classify_concat_parses_and_validates() {
    let combined = format!("{ANALYTIC_SPHERE_WGSL}{CLASSIFY_WGSL}");
    let module = naga::front::wgsl::parse_str(&combined)
        .expect("classify concat should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("classify concat should validate");
}

#[test]
fn classify_wgsl_constants_match_host() {
    // Defaults are the no-feature-flag values (`ROOT_DIM = 16`,
    // `ROOT_CELLS = 4096`) — the WGSL declares them as `override`
    // constants so each `ClassifyPass` pipeline can pin the
    // host-visible values at compile time.
    assert!(
        CLASSIFY_WGSL.contains("override CLASSIFY_ROOT_DIM: u32 = 16u"),
        "CLASSIFY_ROOT_DIM must remain an override defaulting to 16u",
    );
    assert!(
        CLASSIFY_WGSL.contains("override CLASSIFY_ROOT_CELLS: u32 = 4096u"),
        "CLASSIFY_ROOT_CELLS must remain an override defaulting to 4096u",
    );
}

#[test]
fn empty_grid_returns_zero_count() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping empty_grid_returns_zero_count: no GPU");
        return;
    };
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(10_000.0), 1.0);
    let (count, indices) = run_classify_at_lod(&device, &queue, &sampler, DEFAULT_MARGIN, 0);
    assert_eq!(count, 0);
    assert!(indices.is_empty());
}

#[test]
fn sphere_marks_surface_cells() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping sphere_marks_surface_cells: no GPU");
        return;
    };
    let center = Vec3::splat(32.0);
    let radius = 16.0;
    let sampler = AnalyticSphereSampler::new(&device, center, radius);

    let (count, gpu_indices) =
        run_classify_at_lod(&device, &queue, &sampler, DEFAULT_MARGIN, 0);
    let cpu_marks = cpu_classify(&sampler, test_bounds(), DEFAULT_MARGIN);

    assert!(!cpu_marks.is_empty());
    assert_eq!(count as usize, cpu_marks.len());
    let cpu_set: HashSet<u32> = cpu_marks.iter().copied().collect();
    let gpu_set: HashSet<u32> = gpu_indices.iter().copied().collect();
    assert_eq!(gpu_set.len(), count as usize);
    assert_eq!(gpu_set, cpu_set);
}

#[test]
fn classify_skips_when_lod_not_in_mask() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping classify_skips_when_lod_not_in_mask: no GPU");
        return;
    };
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), 16.0);
    let grid = SparseGrid::new(&device, &queue, test_bounds(), 256);

    // Only LOD 0 active → classify at LODs 1, 2, 3 must produce 0
    // marks. LOD 0 itself should mark the surface cells normally.
    queue.write_buffer(grid.chunk_lod_mask_buffer(), 0, &0b0001u32.to_le_bytes());

    let pass = ClassifyPass::new(
        &device,
        sampler.wgsl_source(),
        &sampler.bind_group_layout_entries(),
    );
    let sampler_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::sampler_bg"),
        layout: pass.sampler_bind_group_layout(),
        entries: &sampler.bind_group_entries(),
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test::classify_skip_encoder"),
    });
    for lod_idx in 0..LOD_COUNT {
        pass.record(
            &device, &queue, &mut encoder, &grid, &sampler_bg, lod_idx, DEFAULT_MARGIN,
        );
    }
    queue.submit(std::iter::once(encoder.finish()));

    for lod_idx in 0..LOD_COUNT {
        let count_bytes =
            test_device::readback(&device, &queue, grid.needs_count_buffer(lod_idx));
        let count = u32::from_le_bytes([
            count_bytes[0],
            count_bytes[1],
            count_bytes[2],
            count_bytes[3],
        ]);
        if lod_idx == 0 {
            assert!(
                count > 0,
                "LOD 0 in mask must mark surface cells (got count = 0)",
            );
        } else {
            assert_eq!(
                count, 0,
                "LOD {lod_idx} not in mask must produce 0 marks (got {count})",
            );
        }
    }
}

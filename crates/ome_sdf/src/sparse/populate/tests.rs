//! Tests for [`crate::sparse::populate`] — kept in their own file so
//! the impl module stays under the no-monolithic threshold. GPU tests
//! gate on [`test_device::try_acquire`] and skip when no adapter is
//! available, matching the convention in `classify/tests.rs` and
//! `free_list/tests.rs`.
//!
//! The asserted invariants split three ways:
//!
//! - **Shader-only**: `populate_concat_parses_and_validates` runs naga
//!   parse + validate on the exact concatenation
//!   `freelist + sampler + populate` the pipeline compiles, so a
//!   copy-paste regression in any of the three fragments fails fast
//!   without needing a GPU.
//! - **Functional**: `populate_writes_correct_subgrid_values` and
//!   `populate_idempotent_with_classify` verify the math (sampler
//!   round-trip, no-op re-classify after populate).
//! - **Allocator**: `populate_decrements_free_top_by_marked_count` and
//!   `populate_handles_pool_exhaustion` cover the freelist contract
//!   (S2's pop semantics observed end-to-end through populate).

use super::{POPULATE_WGSL, POPULATE_WORKGROUP_SIZE, PopulatePass};
use crate::sparse::{
    ALLOC_FAILED_SENTINEL, ANALYTIC_SPHERE_WGSL, AnalyticSphereSampler, CLASSIFY_FINALIZE_WGSL,
    ClassifyPass, DEFAULT_MARGIN, EMPTY_ROOT_SENTINEL, ROOT_CELLS, ROOT_DIM, SPARSE_FREELIST_WGSL,
    SUBGRID_DIM, SUBGRID_VOXELS, SdfSampler, SparseGrid, test_device,
};
use glam::Vec3;
use ome_bvh::Aabb;

const TEST_BOUNDS_MIN: Vec3 = Vec3::ZERO;
const TEST_BOUNDS_MAX: Vec3 = Vec3::splat(64.0);

fn test_bounds() -> Aabb {
    Aabb::new(TEST_BOUNDS_MIN, TEST_BOUNDS_MAX)
}

/// Run classify → populate against `sampler` on a fresh grid and read
/// back the four post-pass buffers (root_indices, subgrid_pool,
/// counters, needs_count).
struct PopulateOutput {
    root_indices: Vec<u32>,
    subgrid_pool: Vec<f32>,
    /// `(free_top, alloc_failed_count)` — pads ignored.
    counters: (u32, u32),
    needs_count: u32,
}

fn run_classify_then_populate(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sampler: &AnalyticSphereSampler,
    bounds: Aabb,
    max_subgrids: u32,
) -> (SparseGrid, PopulateOutput) {
    let grid = SparseGrid::new(device, queue, bounds, max_subgrids);
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

    let classify_sampler_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::classify_sampler_bg"),
        layout: classify.sampler_bind_group_layout(),
        entries: &sampler.bind_group_entries(),
    });
    let populate_sampler_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::populate_sampler_bg"),
        layout: populate.sampler_bind_group_layout(),
        entries: &sampler.bind_group_entries(),
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test::populate_encoder"),
    });
    classify.record(
        device,
        queue,
        &mut encoder,
        &grid,
        &classify_sampler_bg,
        DEFAULT_MARGIN,
    );
    populate.record(device, queue, &mut encoder, &grid, &populate_sampler_bg);
    queue.submit(std::iter::once(encoder.finish()));

    let root_bytes = test_device::readback(device, queue, grid.root_indices_buffer());
    let root_indices: Vec<u32> = root_bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let pool_bytes = test_device::readback(device, queue, grid.subgrid_pool_buffer());
    let subgrid_pool: Vec<f32> = pool_bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let counter_bytes = test_device::readback(device, queue, grid.counters_buffer());
    let free_top = u32::from_le_bytes([
        counter_bytes[0],
        counter_bytes[1],
        counter_bytes[2],
        counter_bytes[3],
    ]);
    let alloc_failed = u32::from_le_bytes([
        counter_bytes[4],
        counter_bytes[5],
        counter_bytes[6],
        counter_bytes[7],
    ]);

    let count_bytes = test_device::readback(device, queue, grid.needs_count_buffer());
    let needs_count = u32::from_le_bytes([
        count_bytes[0],
        count_bytes[1],
        count_bytes[2],
        count_bytes[3],
    ]);

    (
        grid,
        PopulateOutput {
            root_indices,
            subgrid_pool,
            counters: (free_top, alloc_failed),
            needs_count,
        },
    )
}

#[test]
fn populate_concat_parses_and_validates() {
    // The exact WGSL the pipeline compiles: freelist helpers + sampler
    // fragment + populate body. Any drift in one fragment's bindings,
    // identifiers, or `var<workgroup>` decls fails here without
    // needing a GPU.
    let combined =
        format!("{SPARSE_FREELIST_WGSL}{ANALYTIC_SPHERE_WGSL}{POPULATE_WGSL}");
    let module = naga::front::wgsl::parse_str(&combined)
        .expect("populate concat should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("populate concat should validate");
}

#[test]
fn populate_finalize_with_override_validates() {
    // Same finalize WGSL classify uses, but standalone — guards
    // against the `override` declaration breaking when consumed
    // without an explicit constant override at parse time.
    let module = naga::front::wgsl::parse_str(CLASSIFY_FINALIZE_WGSL)
        .expect("populate finalize should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("populate finalize should validate");
}

#[test]
fn populate_wgsl_constants_match_host() {
    // Catch silent drift between the WGSL local constants and the host
    // mirrors. Same approach classify_wgsl_constants_match_host uses.
    assert!(
        POPULATE_WGSL.contains(&format!("POPULATE_ROOT_DIM: u32 = {ROOT_DIM}u")),
    );
    assert!(
        POPULATE_WGSL
            .contains(&format!("POPULATE_SUBGRID_DIM: u32 = {SUBGRID_DIM}u")),
    );
    assert!(
        POPULATE_WGSL.contains(&format!(
            "POPULATE_SUBGRID_VOXELS: u32 = {SUBGRID_VOXELS}u",
        )),
    );
    assert!(
        POPULATE_WGSL.contains(&format!(
            "POPULATE_WORKGROUP_SIZE: u32 = {POPULATE_WORKGROUP_SIZE}u",
        )),
    );
}

/// CPU mirror of the populate shader's per-voxel sampling — converts
/// `(cell_idx, voxel_linear_idx)` into the `world_pos` the GPU sampled
/// at that slot. Used by the round-trip test below.
fn voxel_world_pos(cell_idx: u32, voxel_idx: u32, bounds: Aabb) -> Vec3 {
    let cz = cell_idx / (ROOT_DIM * ROOT_DIM);
    let cy = (cell_idx / ROOT_DIM) % ROOT_DIM;
    let cx = cell_idx % ROOT_DIM;
    let extent = bounds.max - bounds.min;
    let cell_size = extent / (ROOT_DIM as f32);
    let cell_min = bounds.min + Vec3::new(cx as f32, cy as f32, cz as f32) * cell_size;

    let vz = voxel_idx / (SUBGRID_DIM * SUBGRID_DIM);
    let vy = (voxel_idx / SUBGRID_DIM) % SUBGRID_DIM;
    let vx = voxel_idx % SUBGRID_DIM;
    let voxel_offset = Vec3::new(vx as f32, vy as f32, vz as f32) / (SUBGRID_DIM as f32);

    cell_min + voxel_offset * cell_size
}

#[test]
fn populate_writes_correct_subgrid_values() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping populate_writes_correct_subgrid_values: no GPU");
        return;
    };
    // Sphere scene with enough surface cells to give the dispatch a
    // non-trivial workload, but small enough to cap the readback at a
    // few hundred KiB.
    // Capacity 1024 — Lipschitz cone with margin 1.0 is generous, the
    // sphere/radius scene marks ~780 cells. 1024 matches the default
    // pool budget and keeps `alloc_failed_count == 0` so the
    // populate-success path is the only thing under test.
    let max_subgrids = 1024u32;
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), 16.0);
    let (_grid, out) = run_classify_then_populate(
        &device, &queue, &sampler, test_bounds(), max_subgrids,
    );

    assert!(out.needs_count > 0, "test scene mis-tuned: no marked cells");
    assert!(
        out.needs_count <= max_subgrids,
        "test scene mis-tuned: needs_count {} exceeds pool capacity {}",
        out.needs_count,
        max_subgrids,
    );
    assert_eq!(
        out.counters.1, 0,
        "no allocations should fail when pool capacity ≥ needs_count",
    );

    // Spot-check every marked cell against the CPU sampler. Each cell
    // owns 4096 voxels; tolerate 1e-5 absolute (well above f32 round-
    // trip error for an analytic sphere on a 64³ chunk).
    let eps = 1e-5_f32;
    let mut checked_cells = 0u32;
    for cell_idx in 0..ROOT_CELLS {
        let root = out.root_indices[cell_idx as usize];
        if root == EMPTY_ROOT_SENTINEL {
            continue;
        }
        assert_ne!(
            root, ALLOC_FAILED_SENTINEL,
            "no cell should hit ALLOC_FAILED with capacity ≥ needs_count",
        );
        assert!(
            root < max_subgrids,
            "root_indices entry must point inside the {max_subgrids}-slot pool",
        );
        let pool_base = (root as usize) * (SUBGRID_VOXELS as usize);
        for voxel_idx in 0..SUBGRID_VOXELS {
            let world_pos = voxel_world_pos(cell_idx, voxel_idx, test_bounds());
            let expected = sampler.sample_cpu(world_pos);
            let actual = out.subgrid_pool[pool_base + voxel_idx as usize];
            assert!(
                (actual - expected).abs() < eps,
                "cell {cell_idx} voxel {voxel_idx}: GPU {actual} vs CPU {expected}",
            );
        }
        checked_cells += 1;
    }
    assert_eq!(
        checked_cells, out.needs_count,
        "every marked cell must have a populated subgrid",
    );
}

#[test]
fn populate_decrements_free_top_by_marked_count() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping populate_decrements_free_top_by_marked_count: no GPU");
        return;
    };
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), 16.0);
    let max_subgrids = 1024u32;
    let (_grid, out) = run_classify_then_populate(
        &device, &queue, &sampler, test_bounds(), max_subgrids,
    );

    assert!(out.needs_count > 0);
    assert!(
        out.needs_count <= max_subgrids,
        "test scene mis-tuned: more marks than pool capacity ({} vs {})",
        out.needs_count,
        max_subgrids,
    );
    assert_eq!(
        out.counters.0,
        max_subgrids - out.needs_count,
        "free_top must decrement by exactly needs_count",
    );
    assert_eq!(out.counters.1, 0, "alloc_failed_count must be zero");
}

#[test]
fn populate_handles_pool_exhaustion() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping populate_handles_pool_exhaustion: no GPU");
        return;
    };
    // Capacity 4 with the sphere scene → needs_count ≫ 4. Exactly 4
    // cells receive a real subgrid index; the rest land on
    // ALLOC_FAILED_SENTINEL (counted by the freelist's
    // alloc_failed_count atomic).
    let max_subgrids = 4u32;
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), 16.0);
    let (_grid, out) = run_classify_then_populate(
        &device, &queue, &sampler, test_bounds(), max_subgrids,
    );

    assert!(
        out.needs_count > max_subgrids,
        "test scene mis-tuned: not enough marked cells to exhaust pool",
    );

    let mut alloced = 0u32;
    let mut failed = 0u32;
    for &root in &out.root_indices {
        if root < max_subgrids {
            alloced += 1;
        } else if root == ALLOC_FAILED_SENTINEL {
            failed += 1;
        }
    }
    assert_eq!(alloced, max_subgrids, "exactly max_subgrids cells must succeed");
    assert_eq!(
        failed,
        out.needs_count - max_subgrids,
        "all overflow cells must land on ALLOC_FAILED_SENTINEL",
    );
    assert_eq!(out.counters.0, 0, "free_top must drain to zero");
    assert_eq!(
        out.counters.1,
        out.needs_count - max_subgrids,
        "alloc_failed_count must equal the overflow",
    );
}

#[test]
fn populate_idempotent_with_classify() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping populate_idempotent_with_classify: no GPU");
        return;
    };
    // First cycle: classify + populate. Then re-run classify alone —
    // because the root indices already point inside the pool, classify
    // must skip every cell and produce needs_count == 0 (the
    // idempotency guard from S3 observed end-to-end through populate).
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), 16.0);
    let (grid, first) = run_classify_then_populate(
        &device, &queue, &sampler, test_bounds(), 1024,
    );

    assert!(first.needs_count > 0, "test scene mis-tuned");
    assert_eq!(first.counters.1, 0, "first cycle should not exhaust pool");

    let classify = ClassifyPass::new(
        &device,
        sampler.wgsl_source(),
        &sampler.bind_group_layout_entries(),
    );
    let sampler_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::idempotent_sampler_bg"),
        layout: classify.sampler_bind_group_layout(),
        entries: &sampler.bind_group_entries(),
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test::idempotent_classify_encoder"),
    });
    classify.record(
        &device,
        &queue,
        &mut encoder,
        &grid,
        &sampler_bg,
        DEFAULT_MARGIN,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let count_bytes = test_device::readback(&device, &queue, grid.needs_count_buffer());
    let second_count = u32::from_le_bytes([
        count_bytes[0],
        count_bytes[1],
        count_bytes[2],
        count_bytes[3],
    ]);
    assert_eq!(
        second_count, 0,
        "after populate, every previously-marked cell points into the pool — \
         classify's idempotency check must skip them all",
    );
}

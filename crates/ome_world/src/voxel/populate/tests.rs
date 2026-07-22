//! Tests for [`crate::voxel::populate`] — kept in their own file so
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
//! - **Allocator**: `populate_allocates_subgrid_per_marked_cell`,
//!   `populate_decrements_free_top_by_marked_count`,
//!   `populate_handles_pool_exhaustion`, and
//!   `populate_idempotent_with_classify` cover the freelist contract
//!   end-to-end through populate.
//! - **Voxel-content semantics** is verified end-to-end by the lookup
//!   tests post-S6 (S6 migrated the pool to a `r16float` texture
//!   atlas; reading values back through the host needs `f16` decoding
//!   we punt on, since `lookup_at_voxel_corners_returns_pool_values`
//!   already proves populate's writes are coherent with the sampler).

use super::{FINALIZE_WGSL, POPULATE_WGSL, POPULATE_WORKGROUP_SIZE, PopulatePass};
use crate::voxel::{
    ALLOC_FAILED_SENTINEL, ANALYTIC_SPHERE_WGSL, AnalyticSphereSampler, ClassifyPass,
    DEFAULT_MARGIN, LOD_COUNT, SPARSE_FREELIST_WGSL, SdfSampler, SparseGrid,
    test_device,
};

/// Sphere radius the populate / lookup tests probe. With the
/// `large-root-grid` feature the root grid quadruples in cell count
/// per axis (32³ vs 16³), so the same sphere shell intersects ~13×
/// more cells. Keeping the marked-cell count under the 1024-slot
/// freelist (so allocation never races against pool exhaustion in
/// invariants tests) needs the radius to scale down accordingly.
#[cfg(not(feature = "large-root-grid"))]
const TEST_SPHERE_RADIUS: f32 = 16.0;
#[cfg(feature = "large-root-grid")]
const TEST_SPHERE_RADIUS: f32 = 8.0;
use glam::Vec3;
use ome_bvh::Aabb;

const TEST_BOUNDS_MIN: Vec3 = Vec3::ZERO;
const TEST_BOUNDS_MAX: Vec3 = Vec3::splat(64.0);

fn test_bounds() -> Aabb {
    Aabb::new(TEST_BOUNDS_MIN, TEST_BOUNDS_MAX)
}

fn enable_all_lods(queue: &wgpu::Queue, grid: &SparseGrid) {
    let all = (1u32 << LOD_COUNT) - 1;
    queue.write_buffer(grid.chunk_lod_mask_buffer(), 0, &all.to_le_bytes());
}

struct PopulateOutput {
    root_indices: Vec<u32>,
    /// `(free_top, alloc_failed_count)` — pads ignored.
    counters: (u32, u32),
    needs_count: u32,
}

/// Run classify → populate at LOD 0 against `sampler` on a fresh grid.
fn run_classify_then_populate(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sampler: &AnalyticSphereSampler,
    bounds: Aabb,
    max_subgrids: u32,
) -> (SparseGrid, PopulateOutput) {
    let grid = SparseGrid::new(device, queue, bounds, max_subgrids);
    enable_all_lods(queue, &grid);
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
        device, queue, &mut encoder, &grid, &classify_sampler_bg, 0, DEFAULT_MARGIN,
    );
    populate.record(device, queue, &mut encoder, &grid, &populate_sampler_bg, 0);
    queue.submit(std::iter::once(encoder.finish()));

    let root_bytes = test_device::readback(device, queue, grid.root_indices_buffer(0));
    let root_indices: Vec<u32> = root_bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let counter_bytes = test_device::readback(device, queue, grid.counters_buffer(0));
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

    let count_bytes = test_device::readback(device, queue, grid.needs_count_buffer(0));
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
            counters: (free_top, alloc_failed),
            needs_count,
        },
    )
}

#[test]
fn populate_concat_parses_and_validates() {
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
    let module = naga::front::wgsl::parse_str(FINALIZE_WGSL)
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
    // Workgroup size stays a `const` — wgpu does not allow overriding
    // `@workgroup_size`. Root dim was promoted to an `override` for
    // the `large-root-grid` feature; the WGSL default reflects the
    // no-feature build.
    assert!(
        POPULATE_WGSL.contains(&format!(
            "POPULATE_WORKGROUP_SIZE: u32 = {POPULATE_WORKGROUP_SIZE}u",
        )),
    );
    assert!(
        POPULATE_WGSL.contains("override POPULATE_ROOT_DIM: u32 = 16u"),
        "POPULATE_ROOT_DIM must remain an override defaulting to 16u",
    );
    // Atlas-geometry overrides must stay declared so the per-LOD host
    // pinning at `PopulatePass::new` succeeds at compile.
    assert!(POPULATE_WGSL.contains("override POPULATE_SUBGRID_DIM"));
    assert!(POPULATE_WGSL.contains("override POPULATE_TILE_DIM"));
    assert!(POPULATE_WGSL.contains("override POPULATE_TILE_VOXELS"));
    assert!(POPULATE_WGSL.contains("override POPULATE_ATLAS_TILES_X"));
    assert!(POPULATE_WGSL.contains("override POPULATE_ATLAS_TILES_Y"));
}

#[test]
fn populate_allocates_subgrid_per_marked_cell() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping populate_allocates_subgrid_per_marked_cell: no GPU");
        return;
    };
    let max_subgrids = 1024u32;
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), TEST_SPHERE_RADIUS);
    let (_grid, out) = run_classify_then_populate(
        &device, &queue, &sampler, test_bounds(), max_subgrids,
    );

    assert!(out.needs_count > 0);
    assert!(out.needs_count <= max_subgrids);
    assert_eq!(out.counters.1, 0);

    let allocated_cells = out
        .root_indices
        .iter()
        .filter(|&&root| root < max_subgrids)
        .count() as u32;
    assert_eq!(allocated_cells, out.needs_count);
    let failed_cells = out
        .root_indices
        .iter()
        .filter(|&&root| root == ALLOC_FAILED_SENTINEL)
        .count() as u32;
    assert_eq!(failed_cells, 0);
}

#[test]
fn populate_decrements_free_top_by_marked_count() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping populate_decrements_free_top_by_marked_count: no GPU");
        return;
    };
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), TEST_SPHERE_RADIUS);
    let max_subgrids = 1024u32;
    let (_grid, out) = run_classify_then_populate(
        &device, &queue, &sampler, test_bounds(), max_subgrids,
    );

    assert!(out.needs_count > 0);
    assert!(out.needs_count <= max_subgrids);
    assert_eq!(out.counters.0, max_subgrids - out.needs_count);
    assert_eq!(out.counters.1, 0);
}

#[test]
fn populate_handles_pool_exhaustion() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping populate_handles_pool_exhaustion: no GPU");
        return;
    };
    let max_subgrids = 4u32;
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), TEST_SPHERE_RADIUS);
    let (_grid, out) = run_classify_then_populate(
        &device, &queue, &sampler, test_bounds(), max_subgrids,
    );

    assert!(out.needs_count > max_subgrids);

    let mut alloced = 0u32;
    let mut failed = 0u32;
    for &root in &out.root_indices {
        if root < max_subgrids {
            alloced += 1;
        } else if root == ALLOC_FAILED_SENTINEL {
            failed += 1;
        }
    }
    assert_eq!(alloced, max_subgrids);
    assert_eq!(failed, out.needs_count - max_subgrids);
    assert_eq!(out.counters.0, 0);
    assert_eq!(out.counters.1, out.needs_count - max_subgrids);
}

#[test]
fn populate_idempotent_with_classify() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping populate_idempotent_with_classify: no GPU");
        return;
    };
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), TEST_SPHERE_RADIUS);
    let (grid, first) = run_classify_then_populate(
        &device, &queue, &sampler, test_bounds(), 1024,
    );

    assert!(first.needs_count > 0);
    assert_eq!(first.counters.1, 0);

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
        &device, &queue, &mut encoder, &grid, &sampler_bg, 0, DEFAULT_MARGIN,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let count_bytes = test_device::readback(&device, &queue, grid.needs_count_buffer(0));
    let second_count = u32::from_le_bytes([
        count_bytes[0],
        count_bytes[1],
        count_bytes[2],
        count_bytes[3],
    ]);
    assert_eq!(second_count, 0);
}

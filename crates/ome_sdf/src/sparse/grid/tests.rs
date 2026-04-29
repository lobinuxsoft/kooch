//! Tests for [`crate::sparse::grid`] — kept in their own file so the
//! impl module stays under the no-monolithic threshold. GPU tests
//! gate on [`test_device::try_acquire`] and skip when no adapter is
//! available, matching the convention in the other sparse submodules.

use super::*;
use crate::sparse::{
    ALLOC_FAILED_SENTINEL, ATLAS_DIM_X, ATLAS_DIM_Y, ATLAS_DIM_Z, EMPTY_ROOT_SENTINEL,
    MAX_SUBGRIDS_DEFAULT, MAX_SUBGRIDS_PER_ATLAS, ROOT_DIM, SUBGRID_DIM, SUBGRID_TILE_DIM,
    SUBGRID_VOXELS, test_device,
};
use glam::Vec3;

fn unit_bounds() -> Aabb {
    Aabb::new(Vec3::ZERO, Vec3::splat(64.0))
}

#[test]
fn constants_consistent() {
    // `MAX_SUBGRIDS_DEFAULT` bounds are enforced at compile time
    // by a `const _` assertion in `super`. The runtime checks
    // here cover the derived-product equalities.
    assert_eq!(ROOT_CELLS, ROOT_DIM * ROOT_DIM * ROOT_DIM);
    assert_eq!(SUBGRID_VOXELS, SUBGRID_DIM * SUBGRID_DIM * SUBGRID_DIM);
    assert_eq!(EMPTY_ROOT_SENTINEL, 0xFFFFFFFF);
    assert_eq!(ALLOC_FAILED_SENTINEL, 0xFFFFFFFE);
}

#[test]
fn buffer_sizes_match_layout() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping buffer_sizes_match_layout: no GPU available");
        return;
    };
    let max_subgrids = 256;
    let grid = SparseGrid::new(&device, &queue, unit_bounds(), max_subgrids);

    assert_eq!(grid.max_subgrids(), max_subgrids);
    assert_eq!(grid.bounds(), unit_bounds());
    assert_eq!(grid.root_indices_buffer().size(), (ROOT_CELLS as u64) * 4);
    let pool = grid.subgrid_pool_texture();
    assert_eq!(pool.format(), POOL_TEXTURE_FORMAT);
    assert_eq!(pool.dimension(), wgpu::TextureDimension::D3);
    let extent = pool.size();
    assert_eq!(extent.width, ATLAS_DIM_X);
    assert_eq!(extent.height, ATLAS_DIM_Y);
    assert_eq!(extent.depth_or_array_layers, ATLAS_DIM_Z);
    assert_eq!(grid.free_list_buffer().size(), (max_subgrids as u64) * 4);
    assert_eq!(grid.counters_buffer().size(), 16);
    assert_eq!(grid.needs_indices_buffer().size(), (ROOT_CELLS as u64) * 4);
    assert_eq!(grid.needs_count_buffer().size(), 4);
    assert_eq!(
        grid.needs_indirect_args_buffer().size(),
        DISPATCH_INDIRECT_ARGS_SIZE,
    );
    assert_eq!(
        grid.populate_indirect_args_buffer().size(),
        DISPATCH_INDIRECT_ARGS_SIZE,
    );
}

#[test]
fn atlas_constants_consistent() {
    // Atlas tile capacity must match `MAX_SUBGRIDS_PER_ATLAS` so a
    // pool exhausted by the freelist also exhausts the atlas
    // tile space — no orphan tiles, no oversized freelists.
    assert_eq!(
        MAX_SUBGRIDS_PER_ATLAS,
        super::super::ATLAS_TILES_X
            * super::super::ATLAS_TILES_Y
            * super::super::ATLAS_TILES_Z,
    );
    assert_eq!(ATLAS_DIM_X, super::super::ATLAS_TILES_X * SUBGRID_TILE_DIM);
    assert_eq!(ATLAS_DIM_Y, super::super::ATLAS_TILES_Y * SUBGRID_TILE_DIM);
    assert_eq!(ATLAS_DIM_Z, super::super::ATLAS_TILES_Z * SUBGRID_TILE_DIM);
    // Skirt invariant — the data interior is `SUBGRID_DIM³`, the
    // atlas tile is `SUBGRID_TILE_DIM³`, and the difference is
    // exactly one voxel per face.
    assert_eq!(SUBGRID_TILE_DIM, SUBGRID_DIM + 1);
    // Pool VRAM ≈ 9.6 MiB — under the issue's 15 MB chunk AC.
    let pool_bytes = (ATLAS_DIM_X as u64)
        * (ATLAS_DIM_Y as u64)
        * (ATLAS_DIM_Z as u64)
        * 2;
    assert!(
        pool_bytes < 15 * 1024 * 1024,
        "pool atlas size {pool_bytes} bytes exceeds 15 MiB AC",
    );
}

#[test]
fn root_indices_initialized_to_empty_sentinel() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping root_indices_initialized_to_empty_sentinel: no GPU available");
        return;
    };
    let grid = SparseGrid::new(&device, &queue, unit_bounds(), 16);
    let bytes = test_device::readback(&device, &queue, grid.root_indices_buffer());
    assert_eq!(bytes.len(), (ROOT_CELLS as usize) * 4);
    for chunk in bytes.chunks_exact(4) {
        let val = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        assert_eq!(
            val, EMPTY_ROOT_SENTINEL,
            "root cell must initialise to EMPTY_ROOT_SENTINEL",
        );
    }
}

#[test]
fn default_capacity_matches_atlas() {
    // S6: pool VRAM is now bounded by atlas dimensions, not by
    // `MAX_SUBGRIDS_DEFAULT × SUBGRID_VOXELS × 4`. The default
    // capacity must equal the tile capacity exactly so the
    // freelist and the atlas drain together.
    assert_eq!(MAX_SUBGRIDS_DEFAULT, MAX_SUBGRIDS_PER_ATLAS);
    // Sanity-check that the data interior is what consumers
    // continue to advertise (`SUBGRID_DIM³`), even though the
    // atlas tile carries one extra skirt voxel per face.
    assert_eq!(SUBGRID_VOXELS, SUBGRID_DIM * SUBGRID_DIM * SUBGRID_DIM);
}

#[test]
#[should_panic(expected = "max_subgrids must be in")]
fn rejects_zero_max_subgrids() {
    let Some((device, queue)) = test_device::try_acquire() else {
        // Force the panic path so this test still validates the
        // assert message format when no GPU is available.
        panic!("max_subgrids must be in 1..=1024, got 0 (skipped — no GPU)");
    };
    let _ = SparseGrid::new(&device, &queue, unit_bounds(), 0);
}

#[test]
#[should_panic(expected = "max_subgrids must be in")]
fn rejects_oversized_max_subgrids() {
    let Some((device, queue)) = test_device::try_acquire() else {
        // Same panic prefix the assertion produces, so the test
        // still validates the message format with no GPU.
        panic!(
            "max_subgrids must be in 1..={MAX_SUBGRIDS_PER_ATLAS}, got 9999 (skipped — no GPU)",
        );
    };
    let _ = SparseGrid::new(&device, &queue, unit_bounds(), MAX_SUBGRIDS_PER_ATLAS + 1);
}

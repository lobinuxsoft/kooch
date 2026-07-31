//! Tests for [`crate::voxel::grid`] — kept in their own file so the
//! impl module stays under the no-monolithic threshold. GPU tests
//! gate on [`test_device::try_acquire`] and skip when no adapter is
//! available, matching the convention in the other sparse submodules.

use super::*;
use crate::voxel::{
    ALLOC_FAILED_SENTINEL, ATLAS_DIM_X, ATLAS_DIM_Y, ATLAS_DIM_Z, EMPTY_ROOT_SENTINEL,
    LOD_COUNT, LOD_LEVELS, MAX_SUBGRIDS_DEFAULT, MAX_SUBGRIDS_PER_ATLAS, ROOT_CELLS,
    ROOT_DIM, SUBGRID_DIM, SUBGRID_TILE_DIM, SUBGRID_VOXELS, test_device,
};
use glam::Vec3;

fn unit_bounds() -> Aabb {
    Aabb::new(Vec3::ZERO, Vec3::splat(64.0))
}

#[test]
fn constants_consistent() {
    assert_eq!(ROOT_CELLS, ROOT_DIM * ROOT_DIM * ROOT_DIM);
    assert_eq!(SUBGRID_VOXELS, SUBGRID_DIM * SUBGRID_DIM * SUBGRID_DIM);
    assert_eq!(EMPTY_ROOT_SENTINEL, 0xFFFFFFFF);
    assert_eq!(ALLOC_FAILED_SENTINEL, 0xFFFFFFFE);
}

#[test]
fn buffer_sizes_match_layout_per_lod() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping buffer_sizes_match_layout_per_lod: no GPU available");
        return;
    };
    let max_subgrids = 256;
    let grid = SparseGrid::new(&device, &queue, unit_bounds(), max_subgrids);

    assert_eq!(grid.max_subgrids(), max_subgrids);
    assert_eq!(grid.bounds(), unit_bounds());

    for lod_idx in 0..LOD_COUNT {
        let lod = LOD_LEVELS[lod_idx as usize];
        assert_eq!(
            grid.root_indices_buffer(lod_idx).size(),
            (ROOT_CELLS as u64) * 4,
            "LOD {lod_idx} root_indices size",
        );
        let pool = grid.subgrid_pool_texture(lod_idx);
        assert_eq!(pool.format(), POOL_TEXTURE_FORMAT);
        assert_eq!(pool.dimension(), wgpu::TextureDimension::D3);
        let extent = pool.size();
        assert_eq!(extent.width, lod.atlas_dim_x, "LOD {lod_idx} atlas width");
        assert_eq!(extent.height, lod.atlas_dim_y, "LOD {lod_idx} atlas height");
        assert_eq!(
            extent.depth_or_array_layers, lod.atlas_dim_z,
            "LOD {lod_idx} atlas depth",
        );
        assert_eq!(
            grid.free_list_buffer(lod_idx).size(),
            (max_subgrids as u64) * 4,
        );
        assert_eq!(grid.counters_buffer(lod_idx).size(), 16);
        assert_eq!(
            grid.needs_indices_buffer(lod_idx).size(),
            (ROOT_CELLS as u64) * 4,
        );
        assert_eq!(grid.needs_count_buffer(lod_idx).size(), 4);
        assert_eq!(
            grid.populate_indirect_args_buffer(lod_idx).size(),
            DISPATCH_INDIRECT_ARGS_SIZE,
        );
    }

    for cascade_idx in 0..(DOWNSAMPLE_CASCADES as u32) {
        assert_eq!(
            grid.downsample_indirect_args_buffer(cascade_idx).size(),
            DISPATCH_INDIRECT_ARGS_SIZE,
        );
    }

    assert_eq!(grid.chunk_lod_mask_buffer().size(), 4);
}

#[test]
fn atlas_constants_consistent() {
    // LOD 0 still shares the module-level constants from S6.
    assert_eq!(
        MAX_SUBGRIDS_PER_ATLAS,
        super::super::ATLAS_TILES_X
            * super::super::ATLAS_TILES_Y
            * super::super::ATLAS_TILES_Z,
    );
    assert_eq!(ATLAS_DIM_X, super::super::ATLAS_TILES_X * SUBGRID_TILE_DIM);
    assert_eq!(ATLAS_DIM_Y, super::super::ATLAS_TILES_Y * SUBGRID_TILE_DIM);
    assert_eq!(ATLAS_DIM_Z, super::super::ATLAS_TILES_Z * SUBGRID_TILE_DIM);
    assert_eq!(SUBGRID_TILE_DIM, SUBGRID_DIM + 1);

    // Total cascade VRAM. AC1 (#136) caps default at 15 MB / chunk;
    // AC4 (#347) caps the `large-root-grid` build at 100 MB / chunk.
    // Lod-level test `total_atlas_vram_under_*` covers the strict
    // budget — the floor here just guarantees we built four atlases.
    let total: u64 = LOD_LEVELS
        .iter()
        .map(|lod| {
            (lod.atlas_dim_x as u64)
                * (lod.atlas_dim_y as u64)
                * (lod.atlas_dim_z as u64)
                * 2
        })
        .sum();
    let cap_bytes: u64 = if cfg!(feature = "large-root-grid") {
        100 * 1024 * 1024
    } else {
        15 * 1024 * 1024
    };
    assert!(
        total < cap_bytes,
        "cascade pool atlas size {total} bytes exceeds cap {cap_bytes}",
    );
}

#[test]
fn root_indices_initialized_to_empty_sentinel_per_lod() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping root_indices_initialized_to_empty_sentinel_per_lod: no GPU");
        return;
    };
    let grid = SparseGrid::new(&device, &queue, unit_bounds(), 16);
    for lod_idx in 0..LOD_COUNT {
        let bytes =
            test_device::readback(&device, &queue, grid.root_indices_buffer(lod_idx));
        assert_eq!(
            bytes.len(),
            (ROOT_CELLS as usize) * 4,
            "LOD {lod_idx} readback size",
        );
        for chunk in bytes.chunks_exact(4) {
            let val = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            assert_eq!(
                val, EMPTY_ROOT_SENTINEL,
                "LOD {lod_idx} root cell must initialise to EMPTY_ROOT_SENTINEL",
            );
        }
    }
}

#[test]
fn default_capacity_matches_atlas() {
    assert_eq!(MAX_SUBGRIDS_DEFAULT, MAX_SUBGRIDS_PER_ATLAS);
    assert_eq!(SUBGRID_VOXELS, SUBGRID_DIM * SUBGRID_DIM * SUBGRID_DIM);
    // All LODs share the same MAX_SUBGRIDS — drains parallel.
    for lod in &LOD_LEVELS {
        assert_eq!(lod.max_subgrids, MAX_SUBGRIDS_PER_ATLAS);
    }
}

#[test]
#[should_panic(expected = "max_subgrids must be in")]
fn rejects_zero_max_subgrids() {
    let Some((device, queue)) = test_device::try_acquire() else {
        panic!("max_subgrids must be in 1..=1024, got 0 (skipped — no GPU)");
    };
    let _ = SparseGrid::new(&device, &queue, unit_bounds(), 0);
}

#[test]
#[should_panic(expected = "max_subgrids must be in")]
fn rejects_oversized_max_subgrids() {
    let Some((device, queue)) = test_device::try_acquire() else {
        panic!(
            "max_subgrids must be in 1..={MAX_SUBGRIDS_PER_ATLAS}, got 9999 (skipped — no GPU)",
        );
    };
    let _ = SparseGrid::new(&device, &queue, unit_bounds(), MAX_SUBGRIDS_PER_ATLAS + 1);
}

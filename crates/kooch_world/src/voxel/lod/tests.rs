use super::*;

/// AC4 of #136 / issue #347 — the `large-root-grid` build must
/// actually deliver `32³ = 32768` root cells with the `(32, 2, 32)`
/// atlas layout. Defended here so a stray edit cannot silently
/// flip the feature back to the 16³ shape.
#[cfg(feature = "large-root-grid")]
#[test]
fn large_root_grid_constants() {
    use super::super::{
        ATLAS_TILES_X, ATLAS_TILES_Y, ATLAS_TILES_Z, MAX_SUBGRIDS_PER_ATLAS, ROOT_CELLS, ROOT_DIM,
    };
    assert_eq!(ROOT_DIM, 32, "large-root-grid must set ROOT_DIM = 32");
    assert_eq!(
        ROOT_CELLS,
        32 * 32 * 32,
        "large-root-grid must give 32768 root cells",
    );
    assert_eq!(ATLAS_TILES_X, 32);
    assert_eq!(ATLAS_TILES_Y, 2, "large-root-grid must double Y to 2");
    assert_eq!(ATLAS_TILES_Z, 32);
    assert_eq!(
        MAX_SUBGRIDS_PER_ATLAS, 2048,
        "large-root-grid atlas must hold 2048 tiles",
    );
    for lod in &LOD_LEVELS {
        assert_eq!(lod.atlas_tiles_y, 2);
        assert_eq!(lod.max_subgrids, 2048);
    }
}

#[test]
fn lod_levels_consistent() {
    // Geometry derived two ways — direct table read vs. recomputed
    // from `subgrid_dim` — must agree.
    for (idx, lod) in LOD_LEVELS.iter().enumerate() {
        assert_eq!(
            lod.tile_dim,
            lod.subgrid_dim + 1,
            "LOD {idx}: tile_dim must be subgrid_dim + 1",
        );
        assert_eq!(
            lod.max_subgrids,
            lod.atlas_tiles_x * lod.atlas_tiles_y * lod.atlas_tiles_z,
            "LOD {idx}: max_subgrids must match tile product",
        );
        assert_eq!(
            lod.voxel_size_factor, LOD_VOXEL_SIZE_FACTORS[idx],
            "LOD {idx}: voxel_size_factor table mismatch",
        );
        assert_eq!(
            lod.voxel_size_factor as f64,
            (1u32 << idx as u32) as f64,
            "LOD {idx}: voxel_size_factor should be 2^lod",
        );
    }
}

#[test]
fn lod_0_matches_module_constants() {
    let lod0 = LOD_LEVELS[0];
    assert_eq!(lod0.subgrid_dim, SUBGRID_DIM);
    assert_eq!(lod0.tile_dim, SUBGRID_TILE_DIM);
    assert_eq!(lod0.atlas_dim_x, ATLAS_DIM_X);
    assert_eq!(lod0.atlas_dim_y, ATLAS_DIM_Y);
    assert_eq!(lod0.atlas_dim_z, ATLAS_DIM_Z);
    assert_eq!(lod0.max_subgrids, MAX_SUBGRIDS_PER_ATLAS);
}

#[cfg(not(feature = "large-root-grid"))]
#[test]
fn total_atlas_vram_under_15_mib() {
    // r16float = 2 bytes per texel.
    let total: u64 = LOD_LEVELS
        .iter()
        .map(|lod| {
            (lod.atlas_dim_x as u64) * (lod.atlas_dim_y as u64) * (lod.atlas_dim_z as u64) * 2
        })
        .sum();
    assert!(
        total < 15 * 1024 * 1024,
        "total LOD atlas VRAM {total} bytes exceeds 15 MiB AC1 (#136)",
    );
    // And — sanity — at least 11 MiB so we know we built the four
    // atlases (LOD 0 alone is ≈9.6 MiB).
    assert!(
        total > 11 * 1024 * 1024,
        "total LOD atlas VRAM {total} bytes suspiciously small",
    );
}

/// AC4 of #136 / issue #347 — 32³ root grid path. Sums the four
/// per-LOD atlases plus the bookkeeping buffers and asserts the
/// total stays inside the `< 100 MiB / chunk` budget.
#[cfg(feature = "large-root-grid")]
#[test]
fn total_atlas_vram_under_100_mib_large_root_grid() {
    let atlases: u64 = LOD_LEVELS
        .iter()
        .map(|lod| {
            (lod.atlas_dim_x as u64) * (lod.atlas_dim_y as u64) * (lod.atlas_dim_z as u64) * 2
        })
        .sum();
    // Bookkeeping per chunk:
    //   4 × root_indices       = 4 × ROOT_CELLS × 4
    //   4 × needs_indices      = 4 × ROOT_CELLS × 4
    //   4 × free_list          = 4 × MAX_SUBGRIDS × 4
    //   4 × counters           = 4 × 16
    //   4 × needs_count        = 4 × 4
    //   4 × populate_indirect  = 4 × 12
    //   3 × downsample_indirect= 3 × 12
    //   1 × chunk_lod_mask     = 4
    //   1 × metrics            = 24
    let root_cells = u64::from(super::super::ROOT_CELLS);
    let max_subgrids = u64::from(super::super::MAX_SUBGRIDS_PER_ATLAS);
    let bookkeeping: u64 = 4 * root_cells * 4
        + 4 * root_cells * 4
        + 4 * max_subgrids * 4
        + 4 * 16
        + 4 * 4
        + 4 * 12
        + 3 * 12
        + 4
        + 24;
    let total = atlases + bookkeeping;
    assert!(
        total < 100 * 1024 * 1024,
        "total per-chunk VRAM {total} bytes exceeds 100 MiB AC4 (atlases {atlases}, bookkeeping {bookkeeping})",
    );
    // Sanity floor — at least 19 MiB so we know LOD 0 atlas is
    // really at the `(544, 34, 544)` shape.
    assert!(
        total > 19 * 1024 * 1024,
        "total per-chunk VRAM {total} bytes suspiciously small for ROOT_DIM=32",
    );
}

#[test]
fn lod_for_voxel_size_buckets_correctly() {
    // cell_size_base = 4.0 → LOD voxel sizes [4, 8, 16, 32].
    let cs = 4.0;
    assert_eq!(lod_for_voxel_size(0.5, cs), 0, "below finest → clamp LOD 0");
    assert_eq!(lod_for_voxel_size(4.0, cs), 0, "exact LOD 0 voxel");
    assert_eq!(lod_for_voxel_size(7.99, cs), 0, "still < LOD 1 voxel");
    assert_eq!(lod_for_voxel_size(8.0, cs), 1, "exact LOD 1 voxel");
    assert_eq!(lod_for_voxel_size(15.99, cs), 1, "between LOD 1 and 2");
    assert_eq!(lod_for_voxel_size(16.0, cs), 2, "exact LOD 2 voxel");
    assert_eq!(lod_for_voxel_size(31.99, cs), 2, "between LOD 2 and 3");
    assert_eq!(lod_for_voxel_size(32.0, cs), 3, "exact LOD 3 voxel");
    assert_eq!(lod_for_voxel_size(1e9, cs), 3, "asymptote → coarsest LOD");
}

#[test]
fn lod_voxel_size_inverts_for_voxel_size() {
    // For every exact-bucket query, lod_voxel_size returns the
    // bucket's voxel pitch.
    let cs = 4.0;
    for lod in 0..LOD_COUNT {
        let pitch = lod_voxel_size(lod, cs);
        assert_eq!(lod_for_voxel_size(pitch, cs), lod);
        // And one ε below the next bucket maps to this LOD too.
        if lod + 1 < LOD_COUNT {
            let next_pitch = lod_voxel_size(lod + 1, cs);
            let just_below = next_pitch - 1.0e-3;
            assert_eq!(lod_for_voxel_size(just_below, cs), lod);
        }
    }
}

#[test]
fn voxels_helpers_match_arithmetic() {
    for lod in &LOD_LEVELS {
        assert_eq!(lod.voxels(), lod.subgrid_dim.pow(3));
        assert_eq!(lod.tile_voxels(), lod.tile_dim.pow(3));
    }
}

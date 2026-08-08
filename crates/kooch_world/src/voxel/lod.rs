//! Per-LOD geometry table for the sparse SDF cascade (issue #136 S7).
//!
//! Each chunk owns four LODs sharing the same root grid (`ROOT_DIM³`)
//! but with progressively coarser subgrids and atlases. Default
//! geometry (no `large-root-grid` feature):
//!
//! ```text
//! LOD 0  →  16³ data + 17³ tile  →  544×17×544 atlas (≈9.6 MiB)
//! LOD 1  →   8³ data +  9³ tile  →  288× 9×288 atlas (≈1.4 MiB)
//! LOD 2  →   4³ data +  5³ tile  →  160× 5×160 atlas (≈ 250 KiB)
//! LOD 3  →   2³ data +  3³ tile  →   96× 3× 96 atlas (≈  53 KiB)
//! ```
//!
//! All LODs share the same `MAX_SUBGRIDS = MAX_SUBGRIDS_PER_ATLAS`
//! (1024 default, 2048 with `large-root-grid`), so freelist +
//! counters + needs-* bookkeeping buffers have identical shapes per
//! LOD. Default total atlas VRAM ≈ 11.3 MiB per chunk — comfortably
//! inside the issue's `< 15 MB / chunk` AC1.
//!
//! With `large-root-grid` enabled, the `Y` axis of every LOD's tile
//! grid doubles (`32 × 2 × 32 = 2048` tiles). Atlas LOD 0 grows to
//! `(544, 34, 544) ≈ 19.2 MiB`; total atlas footprint ≈ 22.7 MiB per
//! chunk, inside the `< 100 MiB / chunk` AC4 budget.
//!
//! # Voxel-size factor
//!
//! `voxel_size_factor[lod]` is the multiplier from one cell's voxel
//! grid pitch to the world-space spacing of voxels in that LOD's tile.
//! Concretely: at LOD `i`, voxel `(vx,vy,vz)` (with `vx ∈ [0, subgrid_dim[i]]`,
//! skirt at `subgrid_dim[i]`) lives at world position
//! `cell_min + (vx,vy,vz) / subgrid_dim[i] * cell_size`. Because
//! `subgrid_dim` halves at each LOD, `voxel_size_factor` doubles —
//! `[1.0, 2.0, 4.0, 8.0]` relative to LOD 0's pitch.
//!
//! # Why fixed-shape (not array<LodConfig>)
//!
//! WGSL has no first-class array of texture bindings, so the lookup
//! and downsample shaders carry one binding per LOD anyway. Mirroring
//! that on the host as `[LodConfig; LOD_COUNT]` keeps the indexing
//! arithmetic constant-foldable in both languages and avoids a `match`
//! over `lod_idx` at every dispatch site.

use super::{
    ATLAS_DIM_X, ATLAS_DIM_Y, ATLAS_DIM_Z, ATLAS_TILES_X, ATLAS_TILES_Y, ATLAS_TILES_Z,
    MAX_SUBGRIDS_PER_ATLAS, SUBGRID_DIM, SUBGRID_TILE_DIM,
};

/// Number of LODs in the cascade. Held as a `u32` so it can be passed
/// to WGSL `override` constants without a host-side cast.
pub const LOD_COUNT: u32 = 4;

/// Geometry parameters for one LOD. Every field is derived from the
/// LOD index — kept materialised here so the host code, the shader
/// override constants, and the test fixtures all read from one source
/// of truth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodConfig {
    /// Side length of the data interior (`subgrid_dim`³ samples per
    /// allocated subgrid). LOD 0 = 16, halves each level.
    pub subgrid_dim: u32,
    /// Side length of the tile in the atlas, including the 1-voxel
    /// skirt per face (`subgrid_dim + 1`).
    pub tile_dim: u32,
    /// Tile counts along each axis. Same `(ATLAS_TILES_X,
    /// ATLAS_TILES_Y, ATLAS_TILES_Z)` triple at every LOD so the
    /// freelist capacity is identical across LODs. Default
    /// `(32, 1, 32)`; `(32, 2, 32)` with `large-root-grid`.
    pub atlas_tiles_x: u32,
    pub atlas_tiles_y: u32,
    pub atlas_tiles_z: u32,
    /// Atlas dimensions in texels (`atlas_tiles_* × tile_dim`).
    pub atlas_dim_x: u32,
    pub atlas_dim_y: u32,
    pub atlas_dim_z: u32,
    /// Maximum subgrids the atlas can hold (= product of tile counts).
    pub max_subgrids: u32,
    /// Voxel pitch multiplier relative to LOD 0. `2^lod` by
    /// construction, materialised so the shader can read it as an
    /// `override` without redoing the exponent.
    pub voxel_size_factor: f32,
}

impl LodConfig {
    /// Voxels per data interior (`subgrid_dim³`).
    pub const fn voxels(&self) -> u32 {
        self.subgrid_dim * self.subgrid_dim * self.subgrid_dim
    }

    /// Voxels per atlas tile, including skirt (`tile_dim³`).
    pub const fn tile_voxels(&self) -> u32 {
        self.tile_dim * self.tile_dim * self.tile_dim
    }
}

/// LOD 0 reuses the existing module-level constants verbatim — every
/// pipeline built before S7 still sees the same shape.
const LOD_0: LodConfig = LodConfig {
    subgrid_dim: SUBGRID_DIM,
    tile_dim: SUBGRID_TILE_DIM,
    atlas_tiles_x: ATLAS_TILES_X,
    atlas_tiles_y: ATLAS_TILES_Y,
    atlas_tiles_z: ATLAS_TILES_Z,
    atlas_dim_x: ATLAS_DIM_X,
    atlas_dim_y: ATLAS_DIM_Y,
    atlas_dim_z: ATLAS_DIM_Z,
    max_subgrids: MAX_SUBGRIDS_PER_ATLAS,
    voxel_size_factor: 1.0,
};

const LOD_1: LodConfig = LodConfig {
    subgrid_dim: 8,
    tile_dim: 9,
    atlas_tiles_x: ATLAS_TILES_X,
    atlas_tiles_y: ATLAS_TILES_Y,
    atlas_tiles_z: ATLAS_TILES_Z,
    atlas_dim_x: ATLAS_TILES_X * 9,
    atlas_dim_y: ATLAS_TILES_Y * 9,
    atlas_dim_z: ATLAS_TILES_Z * 9,
    max_subgrids: MAX_SUBGRIDS_PER_ATLAS,
    voxel_size_factor: 2.0,
};

const LOD_2: LodConfig = LodConfig {
    subgrid_dim: 4,
    tile_dim: 5,
    atlas_tiles_x: ATLAS_TILES_X,
    atlas_tiles_y: ATLAS_TILES_Y,
    atlas_tiles_z: ATLAS_TILES_Z,
    atlas_dim_x: ATLAS_TILES_X * 5,
    atlas_dim_y: ATLAS_TILES_Y * 5,
    atlas_dim_z: ATLAS_TILES_Z * 5,
    max_subgrids: MAX_SUBGRIDS_PER_ATLAS,
    voxel_size_factor: 4.0,
};

const LOD_3: LodConfig = LodConfig {
    subgrid_dim: 2,
    tile_dim: 3,
    atlas_tiles_x: ATLAS_TILES_X,
    atlas_tiles_y: ATLAS_TILES_Y,
    atlas_tiles_z: ATLAS_TILES_Z,
    atlas_dim_x: ATLAS_TILES_X * 3,
    atlas_dim_y: ATLAS_TILES_Y * 3,
    atlas_dim_z: ATLAS_TILES_Z * 3,
    max_subgrids: MAX_SUBGRIDS_PER_ATLAS,
    voxel_size_factor: 8.0,
};

/// Cascade table indexed by LOD. Use [`LOD_LEVELS[lod_idx as usize]`]
/// at host call sites; shader call sites read each field through the
/// `override` constants the host pins per pipeline.
pub const LOD_LEVELS: [LodConfig; LOD_COUNT as usize] = [LOD_0, LOD_1, LOD_2, LOD_3];

/// Voxel-size factors, materialised as a flat array for the lookup
/// shader's `lod_for_voxel_size` helper. Mirrors `voxel_size_factor`
/// across `LOD_LEVELS` — kept in sync by `lod_levels_consistent` in
/// the test module.
pub const LOD_VOXEL_SIZE_FACTORS: [f32; LOD_COUNT as usize] = [1.0, 2.0, 4.0, 8.0];

/// Resolve a target voxel size (in world units) to the LOD index whose
/// voxel pitch best matches it.
///
/// Returns the largest LOD index whose `voxel_size_factor * cell_size_base`
/// is `<= target_voxel_size`. Concretely: if `cell_size_base = 4.0`
/// and `target = 6.0`, LOD 0 voxel = 4.0 (≤6, OK), LOD 1 voxel = 8.0
/// (>6, too coarse) → returns LOD 0. With `target = 16.0`: LOD 0..=2
/// all OK, LOD 3 voxel = 32 (too coarse) → returns LOD 2.
///
/// Edge case: `target_voxel_size < cell_size_base` (asking for finer
/// detail than we have) returns LOD 0 — the finest LOD available.
pub fn lod_for_voxel_size(target_voxel_size: f32, cell_size_base: f32) -> u32 {
    let mut best: u32 = 0;
    let mut i: u32 = 0;
    while i < LOD_COUNT {
        let factor = LOD_VOXEL_SIZE_FACTORS[i as usize];
        if factor * cell_size_base <= target_voxel_size {
            best = i;
        }
        i += 1;
    }
    best
}

/// World-space voxel pitch at `lod_idx` given a base cell size.
/// Inverse of [`lod_for_voxel_size`] (modulo bucketing): the voxel
/// pitch at LOD `i` is `cell_size_base × 2^i`.
pub fn lod_voxel_size(lod_idx: u32, cell_size_base: f32) -> f32 {
    LOD_VOXEL_SIZE_FACTORS[lod_idx as usize] * cell_size_base
}

const _: () = {
    let mut i = 0usize;
    while i < LOD_COUNT as usize {
        let lod = LOD_LEVELS[i];
        // tile_dim must be one larger than subgrid_dim — the skirt
        // invariant carries through every LOD.
        assert!(lod.tile_dim == lod.subgrid_dim + 1);
        // atlas dim = tile_dim × tiles per axis on every axis.
        assert!(lod.atlas_dim_x == lod.tile_dim * lod.atlas_tiles_x);
        assert!(lod.atlas_dim_y == lod.tile_dim * lod.atlas_tiles_y);
        assert!(lod.atlas_dim_z == lod.tile_dim * lod.atlas_tiles_z);
        // max_subgrids must equal the tile count product so the
        // freelist drains exactly when the atlas does — same invariant
        // S6 enforced for LOD 0.
        assert!(lod.max_subgrids == lod.atlas_tiles_x * lod.atlas_tiles_y * lod.atlas_tiles_z,);
        i += 1;
    }
};

#[cfg(test)]
mod tests;

//! Host-side mirrors of the `tile_cull.wgsl` types.
//!
//! `TileBounds` is the per-tile SSBO entry the compute pre-pass writes
//! and the fragment shader reads. `TileCullUniforms` is the small UBO
//! the compute reads to know its viewport dimensions and tile grid.
//! Camera + cascade descriptors are NOT mirrored here — the compute
//! pipeline binds the existing `camera_buffer` and the GDF
//! `frag_uniforms_buffer` directly so a cascade-5 origin update lands
//! in the same frame the tile cull dispatches.
//!
//! Layout pinning lives in the unit tests at the bottom — drift surfaces
//! in CI before reaching the GPU.

use bytemuck::{Pod, Zeroable};

/// Tile workgroup side. 8×8 = 64 threads = single RDNA wavefront, matches
/// the `gdf_populate.wgsl` workgroup tiling so future tile-vs-cascade
/// work-share lands without a layout shuffle.
pub const TILE_WORKGROUP_XY: u32 = 8;

/// Sentinel `t_min` written by the compute when no thread of the tile
/// found a hit and the AABB miss path also fired. Mirrors `1e10` in
/// `tile_cull.wgsl::T_MIN_SENTINEL`. The fragment shader's `flags == 0`
/// gate means this value should never be observed in a non-empty tile.
pub const T_MIN_SENTINEL: f32 = 1.0e10;

/// Bit 0 of `TileBounds::flags` — set when at least one ray of the tile
/// hit a surface inside cascade 5. Cleared tiles short-circuit to
/// `discard` in the fragment shader.
pub const TILE_FLAG_NON_EMPTY: u32 = 1;

/// Per-tile entry in the `tile_ray_bounds` SSBO. 16 bytes — std140-clean
/// for read in the fragment as `array<TileBounds>` storage.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default)]
pub struct TileBounds {
    /// Smallest `t` along any ray of the tile where the coarse cascade
    /// reported a hit (or the cascade-5 AABB enter `t` if the tile
    /// straddles the cascade boundary).
    pub t_min: f32,
    /// Largest `t` along any ray still inside cascade 5. Fragment loop
    /// breaks once `t > t_max` so empty space past the cascade is skipped.
    pub t_max: f32,
    /// Bitfield. Bit 0 = at least one ray of the tile hit a surface
    /// inside cascade 5. Other bits reserved for PR-7+.
    pub flags: u32,
    /// Trailing pad so the entry rounds to 16 bytes (std140 vec4 chunk).
    pub _pad: u32,
}

/// Compute-side UBO. 16 bytes — viewport pixels + tile grid count.
/// The compute reads the camera + GDF cascade descriptors from their
/// own bindings; this struct only carries the per-frame integer pair
/// the dispatch grid depends on.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default, PartialEq, Eq)]
pub struct TileCullUniforms {
    pub viewport_size: [u32; 2],
    pub tile_count: [u32; 2],
}

impl TileCullUniforms {
    /// Tile grid covering `viewport_size`, rounding up so partial-tile
    /// edges still land in the SSBO. `tile_count.x * tile_count.y` is
    /// the SSBO entry count.
    pub fn for_viewport(width: u32, height: u32) -> Self {
        let tile_count_x = width.div_ceil(TILE_WORKGROUP_XY);
        let tile_count_y = height.div_ceil(TILE_WORKGROUP_XY);
        Self {
            viewport_size: [width, height],
            tile_count: [tile_count_x, tile_count_y],
        }
    }

    /// Total tile count for the SSBO sizing.
    pub fn tile_count_total(&self) -> u32 {
        self.tile_count[0] * self.tile_count[1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn tile_bounds_layout() {
        assert_eq!(size_of::<TileBounds>(), 16);
        assert_eq!(align_of::<TileBounds>(), 4);
        assert_eq!(offset_of!(TileBounds, t_min), 0);
        assert_eq!(offset_of!(TileBounds, t_max), 4);
        assert_eq!(offset_of!(TileBounds, flags), 8);
        assert_eq!(offset_of!(TileBounds, _pad), 12);
    }

    #[test]
    fn tile_cull_uniforms_layout() {
        assert_eq!(size_of::<TileCullUniforms>(), 16);
        assert_eq!(align_of::<TileCullUniforms>(), 4);
        assert_eq!(offset_of!(TileCullUniforms, viewport_size), 0);
        assert_eq!(offset_of!(TileCullUniforms, tile_count), 8);
    }

    #[test]
    fn tile_count_rounds_up() {
        // 64x64 viewport, 8-tile -> exact 8x8 grid.
        let u = TileCullUniforms::for_viewport(64, 64);
        assert_eq!(u.tile_count, [8, 8]);
        assert_eq!(u.tile_count_total(), 64);

        // 65x65 -> partial last tile in each axis -> 9x9 grid.
        let u = TileCullUniforms::for_viewport(65, 65);
        assert_eq!(u.tile_count, [9, 9]);

        // 1920x1080 -> 240x135 = 32400 tiles ~= 518 KB ssbo.
        let u = TileCullUniforms::for_viewport(1920, 1080);
        assert_eq!(u.tile_count, [240, 135]);
    }
}

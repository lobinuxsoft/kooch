//! Hi-Z depth-pyramid builder.
//!
//! Builds a max-reduction mip chain over the scene depth buffer for
//! GPU-driven occlusion culling. One [`HiZ`] is created per viewport
//! size; [`HiZ::build_from_depth`] resolves the mip chain into the
//! pyramid texture every frame after the meshlet rasterizer has
//! written depth.
//!
//! # Pipeline
//!
//! ```text
//! depth attachment (Depth32Float, W×H)
//!         │
//!         ▼  cs_copy_depth (compute, 8×8 tiles)
//!  Hi-Z mip 0  (R32Float, W×H)
//!         │
//!         ▼  cs_reduce_max  (compute, 8×8 tiles, dispatch per mip)
//!  Hi-Z mip 1  (R32Float, ⌈W/2⌉×⌈H/2⌉)
//!         │
//!         ▼  cs_reduce_max
//!  Hi-Z mip 2
//!         │
//!         …
//!         ▼
//!  Hi-Z mip N  (1×1 — single texel = max depth of the entire frame)
//! ```
//!
//! Reduction uses a portable `max` of 2×2 — no subgroup ops or LDS
//! tricks (lesson from epic #370 PR-6: portability over single-pass
//! cleverness).

mod builder;

pub use builder::HiZ;

/// Mip count for a `(width, height)` target — `⌈log2(max(w,h))⌉ + 1`,
/// capped at 16. The cap is theoretical headroom — at 16K resolution
/// the chain is 14 mips. Beyond that we stop building.
pub fn mip_count_for(width: u32, height: u32) -> u32 {
    let max_dim = width.max(height).max(1);
    let bits = u32::BITS - (max_dim - 1).leading_zeros();
    (bits + 1).min(MAX_MIP_COUNT)
}

/// Dimensions of mip `level` for a base `(width, height)`. Following
/// `mip_size = max(1, base >> level)` per the wgpu spec. Caller-side
/// out-of-range levels saturate to 1×1 instead of overflowing the
/// shift, so debug-friendly probing past `mip_count` is safe.
pub fn mip_size(width: u32, height: u32, level: u32) -> (u32, u32) {
    let shift = level.min(31);
    ((width >> shift).max(1), (height >> shift).max(1))
}

pub(crate) const MAX_MIP_COUNT: u32 = 16;
pub(crate) const HI_Z_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
pub(crate) const WORKGROUP_SIZE: u32 = 8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_count_for_square_pow2() {
        assert_eq!(mip_count_for(1, 1), 1);
        assert_eq!(mip_count_for(2, 2), 2);
        assert_eq!(mip_count_for(4, 4), 3);
        assert_eq!(mip_count_for(64, 64), 7);
        assert_eq!(mip_count_for(1024, 1024), 11);
    }

    #[test]
    fn mip_count_for_non_pow2_rounds_up() {
        assert_eq!(mip_count_for(3, 3), 3);
        assert_eq!(mip_count_for(5, 7), 4);
        assert_eq!(mip_count_for(1920, 1080), 12);
    }

    #[test]
    fn mip_count_caps_at_max() {
        assert_eq!(mip_count_for(u32::MAX, u32::MAX), MAX_MIP_COUNT);
    }

    #[test]
    fn mip_size_halves_with_floor_min_one() {
        assert_eq!(mip_size(64, 32, 0), (64, 32));
        assert_eq!(mip_size(64, 32, 1), (32, 16));
        assert_eq!(mip_size(64, 32, 5), (2, 1));
        assert_eq!(mip_size(64, 32, 6), (1, 1));
        assert_eq!(mip_size(64, 32, 100), (1, 1));
    }
}

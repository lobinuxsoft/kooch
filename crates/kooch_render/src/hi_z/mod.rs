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

/// SPD's bind-group layout (`hi_z_spd.wgsl`) declares slots `mip_1`
/// through `mip_12`, capping the pyramid at 12 mip levels (max
/// 4096² source). Slots beyond what the actual pyramid uses are
/// bound to a dummy 1×1 texture and the shader early-outs against
/// the `max_mip_level` constant.
pub(crate) const MAX_MIP_COUNT: u32 = 12;
/// SPD writes mips `1..=12` (= up to `MAX_MIP_COUNT` levels), but
/// the shader's bind group always lists 12 storage slots. Anything
/// past `mip_count_for(width, height)` gets bound to a dummy.
pub(crate) const SPD_PYRAMID_SLOT_COUNT: usize = 12;
pub(crate) const HI_Z_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
pub(crate) const WORKGROUP_SIZE: u32 = 8;

#[cfg(test)]
mod tests;

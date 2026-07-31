//! `HiZ` struct: pipelines + mip views + dispatch logic.
//!
//! Production path: SPD (FidelityFX Single-Pass Downsampler) via
//! `cs_downsample_first` + `cs_downsample_second` in `hi_z_spd.wgsl`.
//! Two compute dispatches (wgpu lacks globally coherent storage
//! buffers needed for a true single-pass) but no per-mip
//! storage→texture transitions, which is what unblocks Mesa radv
//! (the per-mip approach this replaces, kept here for tests via
//! `build_from_r32`, hits driver bugs in long editor flows).
//!
//! Test/legacy path: `build_from_r32` keeps the `cs_copy_r32` +
//! `cs_reduce_max` per-mip pipelines from `hi_z_build.wgsl` for the
//! integration tests that pre-fill an R32Float source via
//! `Queue::write_texture` (forbidden on `Depth32Float`). This path
//! is NOT used in production and the dispatchers don't share state
//! with SPD.

mod construction;
mod legacy;
mod spd;
mod types;

#[cfg(test)]
mod tests;

pub use types::HiZ;

const SHADER_SOURCE: &str = include_str!("../../../shaders/hi_z_build.wgsl");
const SPD_SHADER_SOURCE: &str = include_str!("../../../shaders/hi_z_spd.wgsl");

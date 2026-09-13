//! `HiZ` struct: pipelines + mip views + dispatch logic.

mod construction;
mod legacy;
mod spd;
mod types;

#[cfg(test)]
mod tests;

pub use types::HiZ;

const SHADER_SOURCE: &str = include_str!("../../../shaders/hi_z_build.wgsl");
const SPD_SHADER_SOURCE: &str = include_str!("../../../shaders/hi_z_spd.wgsl");

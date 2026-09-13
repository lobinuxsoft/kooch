//! Sky renderer — procedural vertical-gradient background pass.

mod renderer;

pub use renderer::{ActiveSky, SkyRenderPass};

/// Sky shader source (vertex + fragment, fullscreen triangle).
pub(crate) const SHADER_SOURCE: &str = include_str!("../../shaders/sky_main.wgsl");

#[cfg(test)]
mod tests;

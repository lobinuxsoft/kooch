//! Sky renderer — procedural vertical-gradient background pass.

mod graph;
mod preview;
mod renderer;

pub use preview::SkyPreview;
pub use renderer::{ActiveSky, SkyRenderPass, SkyTarget};

/// Sky shader source (vertex + fragment, fullscreen triangle).
pub(crate) const SHADER_SOURCE: &str = include_str!("../../shaders/sky_main.wgsl");

#[cfg(test)]
mod tests;

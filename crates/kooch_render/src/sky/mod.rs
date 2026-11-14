//! Sky renderer: a fullscreen `kind: sky` graph behind the scene.

mod default;
mod graph;
mod preview;
mod renderer;

pub use default::DEFAULT_SKY_SHADER;
pub use preview::SkyPreview;
pub use renderer::{ActiveSky, SkyRenderPass, SkyTarget};

#[cfg(test)]
mod tests;

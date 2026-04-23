//! ome_render — renderers for oh_my_engine.
//!
//! - [`RenderPlugin`] clears the screen with a solid [`ClearColor`] each
//!   frame (used as the minimal GPU smoke test).
//! - [`RayMarchPlugin`] sphere-traces SDF components from the ECS into
//!   a fullscreen fragment shader.
//! - [`MeshPassRenderer`] draws every visible `MeshRenderer + GlobalTransform`
//!   entity using glTF-loaded meshes, layered on top of the SDF image.

pub mod clear_color;
pub mod fps;
pub mod mesh;
pub mod plugin;
pub mod raymarch;
pub mod raymarch_plugin;
pub mod sky;
mod systems;

/// Depth format shared by every renderer that writes into the editor's
/// offscreen viewport target. `Depth32Float` is universally supported
/// and gives enough precision without stencil (which we don't use).
pub const VIEWPORT_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub use clear_color::ClearColor;
pub use fps::FpsTracker;
pub use mesh::{Aabb, GpuMesh, MeshLoadError, MeshLoader, MeshPassRenderer, MeshVertex};
pub use plugin::RenderPlugin;
pub use raymarch::{RayMarchParams, RayMarchRenderer};
pub use raymarch_plugin::{RayMarchPlugin, SkyGradient};
pub use sky::{ActiveSky, SkyRenderPass};

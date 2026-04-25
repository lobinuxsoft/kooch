//! ome_render — renderers for oh_my_engine.
//!
//! - [`RenderPlugin`] is the full game render pipeline (sky + raymarch +
//!   mesh) targeting the swapchain surface. Used by play-mode binaries
//!   via `oh_my_engine::DefaultPlugins`.
//! - [`RayMarchPlugin`] is a minimal sphere-tracing-only pipeline used by
//!   the `raymarch_demo` example.
//! - [`MeshPassRenderer`], [`RayMarchRenderer`] and [`SkyRenderPass`] are
//!   the underlying renderers, also reused by the editor's offscreen
//!   viewport orchestrator.

pub mod fps;
pub mod gizmos;
pub mod mesh;
pub mod plugin;
pub mod raymarch;
pub mod raymarch_plugin;
pub mod sky;

/// Depth format shared by every renderer that writes into the editor's
/// offscreen viewport target. `Depth32Float` is universally supported
/// and gives enough precision without stencil (which we don't use).
pub const VIEWPORT_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub use fps::FpsTracker;
pub use gizmos::{GizmoBatch, GizmoRenderer, LineSegment};
pub use mesh::{Aabb, GpuMesh, MeshLoadError, MeshLoader, MeshPassRenderer, MeshVertex};
pub use plugin::RenderPlugin;
pub use raymarch::{RayMarchParams, RayMarchRenderer};
pub use raymarch_plugin::{RayMarchPlugin, SkyGradient};
pub use sky::{ActiveSky, SkyRenderPass};

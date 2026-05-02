//! ome_render — renderers for oh_my_engine.
//!
//! Post-pivot 2026-05-02 (plan C): mesh-only render path. SDF render
//! (raymarch + tile-cull + GDF) was deleted; SDF brushes are preserved
//! upstream in `ome_sdf` to feed the future Phase 2.5 voxel + DC pipeline.
//!
//! - [`RenderPlugin`] is the full game render pipeline (sky + mesh)
//!   targeting the swapchain surface. Used by play-mode binaries via
//!   `oh_my_engine::DefaultPlugins`.
//! - [`MeshPassRenderer`] and [`SkyRenderPass`] are the underlying
//!   renderers, reused by the editor's offscreen viewport orchestrator.
//!
//! Gizmo rendering lives in the dedicated `ome_gizmos` crate.

pub mod fps;
pub mod mesh;
pub mod plugin;
pub mod sky;
pub mod texture;

/// Depth format shared by every renderer that writes into the editor's
/// offscreen viewport target. `Depth32Float` is universally supported
/// and gives enough precision without stencil (which we don't use).
pub const VIEWPORT_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub use fps::FpsTracker;
pub use mesh::{Aabb, GpuMesh, MeshLoadError, MeshLoader, MeshPassRenderer, MeshVertex};
pub use plugin::RenderPlugin;
pub use sky::{ActiveSky, SkyRenderPass};
pub use texture::{GpuTexture, Image, ImageFormat, ImageLoader};

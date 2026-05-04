//! ome_render — renderers for oh_my_engine.
//!
//! Post-pivot 2026-05-02 (plan C): mesh-only render path. SDF render
//! (raymarch + tile-cull + GDF) was deleted; SDF brushes are preserved
//! upstream in `ome_sdf` to feed the future Phase 2.5 voxel + DC pipeline.
//!
//! - [`RenderPlugin`] is the full game render pipeline (sky + meshlet
//!   GPU-driven cull/raster/shade) targeting the swapchain surface. Used
//!   by play-mode binaries via `oh_my_engine::DefaultPlugins`.
//! - [`SkyRenderPass`] and the `meshlet` module's `MeshletRenderStage` /
//!   `MeshletBlit` are reused by the editor's offscreen viewport
//!   orchestrator.
//!
//! Gizmo rendering lives in the dedicated `ome_gizmos` crate.

pub mod fps;
pub mod graph;
pub mod hi_z;
pub mod material;
pub mod mesh;
pub mod meshlet;
pub mod plugin;
pub mod sky;
pub mod texture;

/// Depth format shared by every renderer that writes into the editor's
/// offscreen viewport target. `Depth32Float` is universally supported
/// and gives enough precision without stencil (which we don't use).
pub const VIEWPORT_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub use fps::FpsTracker;
pub use graph::{FnNode, FrameInfo, GraphError, NodeId, RenderContext, RenderGraph, RenderNode};
pub use hi_z::HiZ;
pub use material::{MaterialParams, MaterialPool};
pub use mesh::{Aabb, MeshVertex};
pub use meshlet::{
    build_default_meshlets, build_meshlets_from_mesh, MeshletBuildError, MeshletDescriptor,
    MeshletMesh,
};
pub use plugin::RenderPlugin;
pub use sky::{ActiveSky, SkyPassNode, SkyRenderPass};
pub use texture::{GpuTexture, Image, ImageFormat, ImageLoader};

//! kooch_render — renderers for kooch.
//!
//! Post-pivot 2026-05-02 (plan C): mesh-only render path. SDF render
//! (raymarch + tile-cull + GDF) was deleted. The CSG brush primitives
//! that fed it went with it in 2026-07; what survived is the sparse
//! voxel storage, now `kooch_world::voxel`, which the Phase 2.5 dual
//! contouring pipeline extracts meshes from.
//!
//! - [`RenderPlugin`] is the full game render pipeline (sky + meshlet
//!   GPU-driven cull/raster/shade) targeting the swapchain surface. Used
//!   by play-mode binaries via `kooch::DefaultPlugins`.
//! - [`SkyRenderPass`] and the `meshlet` module's `MeshletRenderStage` /
//!   `MeshletBlit` are reused by the editor's offscreen viewport
//!   orchestrator.
//!
//! Gizmo rendering lives in the dedicated `kooch_gizmos` crate.

pub mod graph;
pub mod hi_z;
pub mod material;
pub mod mesh;
pub mod meshlet;
pub mod perf;
pub mod plugin;
pub mod projection;
pub mod settings;
pub mod shadow;
pub mod sky;
pub mod texture;
pub mod vbuf64;
pub mod view_camera;

/// Depth format shared by every renderer that writes into the editor's
/// offscreen viewport target. `Depth32Float` is universally supported
/// and gives enough precision without stencil (which we don't use).
pub const VIEWPORT_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub use graph::{FnNode, FrameInfo, GraphError, NodeId, RenderContext, RenderGraph, RenderNode};
pub use hi_z::HiZ;
pub use material::{MaterialParams, MaterialPool};
pub use mesh::{Aabb, MeshVertex};
pub use meshlet::{
    MeshletBuildError, MeshletDescriptor, MeshletMesh, build_default_meshlets,
    build_meshlets_from_mesh,
};
pub use perf::EngineVramTracker;
pub use plugin::RenderPlugin;
pub use projection::perspective_rh_reverse_z;
pub use sky::{ActiveSky, SkyRenderPass};
pub use texture::{GpuTexture, Image, ImageFormat, ImageLoader};
pub use vbuf64::{
    CLUSTER_ID_BITS, MAX_CLUSTER_ID, TRI_ID_BITS, TRI_ID_MASK, Vbuf64Support, pack_visibility,
    unpack_visibility,
};

pub use view_camera::ViewCamera;

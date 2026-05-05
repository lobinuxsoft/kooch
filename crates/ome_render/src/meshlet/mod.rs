//! Meshlet pipeline — virtual geometry foundation for Phase 1.D (#117).
//!
//! A "meshlet" is a small cluster of triangles (up to 64 vertices + 124
//! triangles in our default config) with associated bounds + a normal
//! cone for backface culling. Modern GPU-driven renderers operate at
//! meshlet granularity instead of mesh granularity:
//!
//! - **Frustum + occlusion culling** runs in compute, one thread per
//!   meshlet (millions of meshlets → millions of threads, GPU-native).
//! - **LOD selection** picks a different meshlet per chunk based on
//!   screen-space size, transition is invisible (DAG hierarchy).
//! - **Indirect draw / mesh shaders** consume the surviving meshlets
//!   without CPU-GPU round-trip.
//!
//! # Scope (PR-1 of #117)
//!
//! Covered:
//! - [`MeshletMesh`] CPU asset (vertex pool + meshlet array + per-meshlet
//!   metadata: bounds, normal cone)
//! - Offline builder: `Mesh` → `MeshletMesh` via `meshopt::build_meshlets`
//! - AABB + cone culling data per meshlet
//!
//! Deferred (separate PRs of #117):
//! - GPU-side `MeshletMesh` upload (storage buffers)
//! - Compute culling shader (frustum + cone + Hi-Z)
//! - Indirect draw integration
//! - Visibility buffer + deferred shading
//! - LOD DAG (cluster hierarchy)
//! - Mesh shader path (when `Features::EXPERIMENTAL_MESH_SHADER` lands
//!   on every backend we target)

mod asset;
mod blit;
mod builder;
mod cull;
mod debug;
mod deferred;
mod dispatcher;
mod drawer;
mod gpu_meshlet;
mod loader;
mod pool;
mod render_stage;
mod scene;
mod system;
mod vis_buffer;

pub use asset::{
    MeshletDescriptor, MeshletMesh, DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES,
    MESHLET_ROOT_PARENT,
};
pub use blit::MeshletBlit;
pub use builder::{
    build_default_meshlets, build_meshlets_from_mesh, build_meshlets_lod_chain, LodConfig,
    MeshletBuildError,
};
pub use cull::{
    camera_in_backface_cone, extract_frustum_planes, sphere_outside_frustum, CullParams,
};
pub use debug::MeshletDebugMode;
pub use deferred::{MeshletDeferredShader, DEFERRED_COLOR_FORMAT};
pub use dispatcher::{DrawIndirectArgs, HiZTestParams, MeshletCull};
pub use drawer::MeshletDrawer;
pub use gpu_meshlet::{
    binding, meshlet_bind_group, meshlet_bind_group_layout, pool_meshlet_bind_group,
    GpuMeshletMesh,
};
pub use loader::MeshletMeshLoader;
pub use pool::{GlobalMeshPool, GpuGlobalMeshPool, MeshDescriptor, MeshHandle};
pub use render_stage::{
    MeshletRenderStage, MeshletRenderStageConfig, MeshletRenderStats,
};
pub use scene::{
    decode_scene_visible_id, encode_scene_visible_id, MeshInstance, MeshletScene,
    SceneCullParams,
};
pub use system::{instance_at_origin, MeshletPipeline};
pub use vis_buffer::{MeshletVisRasterizer, VISIBILITY_BUFFER_FORMAT};

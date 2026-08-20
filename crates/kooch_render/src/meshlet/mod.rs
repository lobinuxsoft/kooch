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
mod caps;
mod cull;
mod debug;
pub(crate) mod deferred;
mod dispatcher;
mod drawer;
mod gpu_meshlet;
mod gpu_timers;
mod loader;
mod material_pass;
mod pool;
mod reject_overlay;
mod render_stage;
mod scene;
mod stage_counters;
mod system;
mod vbuf64_stage;
mod vis_buffer;

pub use asset::{
    DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES, MESHLET_GROUP_NONE, MESHLET_ROOT_PARENT,
    MeshletDescriptor, MeshletMesh,
};
pub use blit::MeshletBlit;
pub use builder::{
    LodConfig, MeshletBuildError, build_default_meshlets, build_meshlets_from_mesh,
    build_meshlets_lod_chain,
};
pub use caps::MeshletDebugCaps;
pub use cull::{
    CullParams, camera_in_backface_cone, extract_frustum_planes, projection_scale_y,
    sphere_outside_frustum,
};
pub use debug::{MeshletDebugMode, MeshletLodSettings};
pub use deferred::{DEFERRED_COLOR_FORMAT, MeshletDeferredShader};
pub use dispatcher::{DrawIndirectArgs, HiZTestParams, MeshletCull, MeshletCullPipelines};
pub use drawer::MeshletDrawer;
pub use gpu_meshlet::{
    GpuMeshletMesh, binding, meshlet_bind_group, meshlet_bind_group_layout, pool_meshlet_bind_group,
};
pub use gpu_timers::MeshletGpuTimers;
pub use loader::MeshletMeshLoader;
pub use material_pass::{
    MATERIAL_DEPTH_FORMAT, MATERIAL_PASS_CONTACT_DEPTH_BINDING, MATERIAL_PASS_CONTACT_UBO_BINDING,
    MATERIAL_PASS_INTI_GROUP, MATERIAL_PBR_COMPUTE_BODY, MATERIAL_PBR_DEFAULT_BODY,
    RESOLVE_MATERIAL_DEPTH_SHADER, SHADING_TILE_SIZE, SURFACE_RECONSTRUCT_SHADER,
    VISIBILITY_BUFFER_RESOLVE_SHADER, compose_material_shader,
};
pub use pool::{GlobalMeshPool, GpuGlobalMeshPool, MeshBounds, MeshDescriptor, MeshHandle};
pub use reject_overlay::{MeshletRejectOverlay, RejectReason};
pub use render_stage::{MeshletRenderStage, MeshletRenderStageConfig, MeshletRenderStats, ViewId};
pub use scene::{
    MeshInstance, MeshletScene, SceneCullParams, decode_scene_visible_id, encode_scene_visible_id,
};
pub use stage_counters::{CullStageCounts, MeshletStageCounters};
pub use system::{MeshletPipeline, instance_at_origin};
pub use vbuf64_stage::{JITTER_BASE_PHASES, Jitter, ShadingRate, Vbuf64Stage};

/// `KOOCH_COMPUTE_SHADING`, when it says anything. See
/// [`crate::quality`] for why the variable outranks the settings asset.
pub fn compute_shading_override() -> Option<bool> {
    vbuf64_stage::compute_shading_override()
}

/// `KOOCH_SHADING_RATE`, when it says anything.
pub fn shading_rate_override() -> Option<ShadingRate> {
    vbuf64_stage::shading_rate_override()
}
pub use vis_buffer::{MeshletVisRasterizer, VISIBILITY_BUFFER_FORMAT};

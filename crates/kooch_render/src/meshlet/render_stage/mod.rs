//! End-to-end meshlet render stage — Phase 1.E.3c orchestrator.
//!
//! Owns the full per-frame meshlet pipeline state and runs the
//! cull → vbuf → deferred chain off ECS data:
//!
//! ```text
//! Resources (MeshRenderer + GlobalTransform + Assets<MeshletMesh>)
//!         │  collect_scene_instances
//!         ▼
//! Vec<MeshInstance>  ──► MeshletScene.upload_instances
//!         │
//!         ▼
//! MeshletCull.dispatch_scene_pool  (one dispatch over the entire GpuGlobalMeshPool)
//!         │
//!         ▼
//! MeshletVisRasterizer.render_scene  (R32Uint visibility buffer + depth)
//!         │
//!         ▼
//! MeshletDeferredShader.shade_scene  (compute → Rgba8Unorm color view)
//! ```
//!
//! # Multi-mesh path (#446 / #457)
//!
//! [`Self::ensure_gpu_mesh`] registers a `MeshletMesh` into the
//! [`MeshletPipeline`]'s `GlobalMeshPool` and marks the GPU mirror
//! dirty. [`Self::render_with_assets`] rebuilds the
//! [`GpuGlobalMeshPool`] when dirty, then dispatches
//! `cs_cull_scene_pool` over every (instance, meshlet) pair across
//! the whole pool — one cull dispatch per frame regardless of how
//! many distinct meshes the scene references.
//!
//! # Owning vs borrowing
//!
//! The stage *owns* the visibility buffer / depth / color textures so
//! the same allocations survive across frames. The plugin layer
//! (1.E.3b) will hand the color view back to the editor's offscreen
//! target via a copy or by binding the stage's view directly.

mod config;
mod frame;
mod helpers;
mod new;
mod stage;
mod stats;
mod view_targets;

#[cfg(test)]
mod tests;

pub use config::MeshletRenderStageConfig;
pub use stage::{MeshletRenderStage, ViewId};
pub use stats::MeshletRenderStats;

pub(crate) use helpers::{create_2d_attachment, depth_sample_view};

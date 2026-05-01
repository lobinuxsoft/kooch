//! CPU bridge between [`ome_world::ChunkManager`] and the GPU pool.
//!
//! `ChunkContentSource` is by design CPU-only — it never sees a
//! `wgpu::Queue` or `OmeAccel`. The streaming layer surfaces its
//! per-frame delta via [`ChunkManager::drain_pending_loads`] /
//! [`ChunkManager::drain_pending_unloads`]; this module mirrors that
//! delta into the renderer's [`crate::raymarch::bvh::BvhState`].
//!
//! Lives in its own file so the legacy ECS scene-collection path in
//! `update.rs` stays under the workspace's 400-LoC monolith cap.

use ome_core::resource::Resources;
use ome_world::ChunkManager;

use super::renderer::RayMarchRenderer;

impl RayMarchRenderer {
    /// Mirror the world streaming layer's pending load/unload delta
    /// into the GPU pool. Drains `ChunkManager`'s pending queues; the
    /// streaming layer never sees `wgpu::Queue` (DOD: the trait
    /// boundary is CPU-only), so this bridge is the renderer's job.
    ///
    /// Run **before** the legacy ECS-driven single-chunk pass so the
    /// pool's TLAS rebuild inside `update_gpu` reflects the new
    /// topology before the renderer continues. Errors are logged at
    /// `warn` and absorbed — a single failed insert/remove must not
    /// poison the rest of the per-frame delta.
    pub fn apply_streaming_delta(&mut self, queue: &wgpu::Queue, resources: &mut Resources) {
        let Some(mut manager) = resources.remove::<ChunkManager>() else {
            return;
        };

        let unloads = manager.drain_pending_unloads();
        let loads = manager.drain_pending_loads();
        let unload_count = unloads.len();
        let load_count = loads.len();

        for chunk_id in unloads {
            if let Err(e) = self.bvh_state.remove_streaming_chunk(queue, chunk_id) {
                tracing::warn!(
                    target: "ome_render::raymarch",
                    chunk = ?chunk_id,
                    "remove_streaming_chunk failed: {e}",
                );
            }
        }
        for (chunk_id, content) in loads {
            if let Err(e) = self.bvh_state.insert_streaming_chunk(queue, chunk_id, &content) {
                tracing::warn!(
                    target: "ome_render::raymarch",
                    chunk = ?chunk_id,
                    primitives = content.primitives.len(),
                    "insert_streaming_chunk failed: {e}",
                );
            }
        }

        if load_count > 0 || unload_count > 0 {
            tracing::info!(
                target: "ome_render::raymarch",
                loads = load_count,
                unloads = unload_count,
                streaming_chunks = self.bvh_state.streaming_chunk_count(),
                streaming_prims = self.bvh_state.total_primitive_count(),
                "applied streaming delta",
            );
        }

        resources.insert(manager);
    }

    /// Read-only view of the renderer's BVH pool state. Used by the
    /// editor's viewport gate to decide whether the raymarch pass needs
    /// to run (a project may have streaming chunks resident with no
    /// ECS-side SDFs at all).
    pub fn bvh_state(&self) -> &super::bvh::BvhState {
        &self.bvh_state
    }

    /// Mutable handle to the BVH pool. The headless render-correctness
    /// AC test in `tests/raymarch_renders_sphere.rs` uses this to seed
    /// a one-sphere scene without going through the ECS streaming
    /// path; production code stays on `apply_streaming_delta` and
    /// `update_scene` instead.
    pub fn bvh_state_mut(&mut self) -> &mut super::bvh::BvhState {
        &mut self.bvh_state
    }

    /// Write raw bytes into the per-frame camera uniforms buffer.
    /// Lower-level than [`Self::update_camera`], which derives the
    /// uniforms from the ECS camera entity. The headless render AC
    /// test populates the buffer directly because there is no ECS
    /// world in scope.
    pub fn write_camera_uniforms(&self, queue: &wgpu::Queue, bytes: &[u8]) {
        queue.write_buffer(&self.camera_buffer, 0, bytes);
    }

    /// Write raw bytes into the per-frame scene-meta uniforms buffer
    /// (sky colours + `skip_internal_sky` + per-role smoothness
    /// summaries). Same lower-level role as
    /// [`Self::write_camera_uniforms`].
    pub fn write_scene_meta(&self, queue: &wgpu::Queue, bytes: &[u8]) {
        queue.write_buffer(&self.scene_meta_buffer, 0, bytes);
    }

    /// Push the current `self.params` (`max_steps`, `max_distance`,
    /// `surface_threshold`, `epsilon_factor`) into the GPU uniform
    /// buffer. Production code lets `update_camera` flush the params
    /// every frame; tests that bypass the ECS path call this after
    /// mutating `params` so the override actually reaches the shader.
    pub fn write_raymarch_params(&self, queue: &wgpu::Queue) {
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&self.params));
    }
}

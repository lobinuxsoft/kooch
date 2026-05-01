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
            tracing::debug!(
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
}

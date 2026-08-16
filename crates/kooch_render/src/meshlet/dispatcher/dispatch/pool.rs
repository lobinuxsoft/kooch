use crate::meshlet::cull::CullParams;
use crate::meshlet::pool::GpuGlobalMeshPool;
use crate::meshlet::scene::{MeshletScene, SceneCullParams};

use super::super::MeshletCull;
use super::super::pipelines::MeshletCullPipelines;

impl MeshletCull {
    /// Multi-mesh scene cull (#446). One dispatch covers every
    /// instance × meshlet across the entire [`GpuGlobalMeshPool`].
    /// Visible pairs land in `visible_meshlets[]` packed as
    /// `(instance_id << 16) | global_meshlet_idx` (pool-relative —
    /// not per-mesh as in [`Self::dispatch_scene`]).
    ///
    /// # Capacity
    ///
    /// `MeshletCull::capacity` must cover the worst-case dispatch
    /// (`instance_count × pool.max_meshlets_per_mesh`). Per-thread
    /// bounds checks against the actual mesh's meshlet_count keep
    /// shorter meshes from over-running, but the capacity must still
    /// cover the rectangular thread grid.
    ///
    /// `mesh_count_for_dispatch` is the worst-case meshlet stride —
    /// pass `pool.max_meshlets_per_mesh()`. The shader bounds-checks
    /// per-instance via `mesh_descriptors[mesh_id].meshlet_count`.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_scene_pool(
        &self,
        pipelines: &MeshletCullPipelines,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &GpuGlobalMeshPool,
        scene: &MeshletScene,
        cull_params: &CullParams,
        scene_params: &SceneCullParams,
    ) {
        let total_threads = scene_params.instance_count * scene_params.meshlets_per_mesh;
        debug_assert!(
            total_threads <= self.capacity,
            "scene pool total (instances × max_meshlets/mesh) {} exceeds dispatcher capacity {}",
            total_threads,
            self.capacity,
        );
        debug_assert!(
            pool.mesh_count > 0,
            "dispatch_scene_pool called with an empty pool",
        );

        let params_binding = self.stage_params(queue, cull_params);
        queue.write_buffer(
            &self.scene_params_buffer,
            0,
            bytemuck::bytes_of(scene_params),
        );
        encoder.clear_buffer(&self.visible_count, 0, None);

        // The pool path leaves the single-mesh `descriptors` binding
        // unbound at the WGSL level (Naga drops it from the entry's
        // required set), but the BGL still requires a valid resource
        // there. Bind the pool's meshlets buffer as a placeholder —
        // the shader simply does not read from it.
        let cull_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_scene_pool_cull_bg"),
            layout: &pipelines.cull_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(params_binding.clone()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pool.meshlets.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.visible_meshlets.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.visible_count.as_entire_binding(),
                },
            ],
        });
        // Two-entry bind group matching the cull-only pool BGL (the
        // full GpuGlobalMeshPool::bind_group covers the rasterizer's
        // 5-entry layout).
        let pool_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_scene_pool_pool_bg"),
            layout: &pipelines.pool_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pool.mesh_descriptors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pool.meshlets.as_entire_binding(),
                },
            ],
        });
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_scene_pool_scene_bg"),
            layout: &pipelines.scene_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: scene.instance_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.scene_params_buffer.as_entire_binding(),
                },
            ],
        });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_scene_pool_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.pipeline_scene_pool);
            pass.set_bind_group(0, &cull_bg, &[]);
            pass.set_bind_group(1, &pool_bg, &[]);
            pass.set_bind_group(2, &scene_bg, &[]);
            let workgroups = total_threads.div_ceil(64).max(1);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        self.mirror_count_to_indirect_args(encoder);
    }
}

use crate::meshlet::cull::CullParams;
use crate::meshlet::pool::GpuGlobalMeshPool;
use crate::meshlet::scene::{MeshletScene, SceneCullParams};
use kooch_core::gpu::tiled_workgroups;

use super::super::MeshletCull;
use super::super::pipelines::MeshletCullPipelines;

impl MeshletCull {
    /// Multi-mesh scene cull (#446). One dispatch covers every instance × meshlet across the entire
    /// [`GpuGlobalMeshPool`]. Visible pairs land in `visible_meshlets[]` packed as `(instance_id <<
    /// 16) | global_meshlet_idx` (pool-relative — not per-mesh as in [`Self::dispatch_scene`]).
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

        let params = self.stage_params(queue, cull_params);
        self.stage_scene(queue, scene_params);
        encoder.clear_buffer(&self.visible_count, 0, None);

        let binds = self.binds(pipelines, device, pool, scene);

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_scene_pool_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.pipeline_scene_pool);
            pass.set_bind_group(0, &binds.cull, &[params]);
            pass.set_bind_group(1, &binds.pool, &[]);
            pass.set_bind_group(2, &binds.scene, &[]);
            let (groups_x, groups_y) = tiled_workgroups(total_threads, 64);
            pass.dispatch_workgroups(groups_x, groups_y, 1);
        }

        self.mirror_count_to_indirect_args(encoder);
    }
}

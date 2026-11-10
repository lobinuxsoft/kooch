use crate::meshlet::cull::CullParams;
use crate::meshlet::pool::GpuGlobalMeshPool;
use crate::meshlet::scene::{MeshletScene, SceneCullParams};
use kooch_core::gpu::tiled_workgroups;

use super::super::MeshletCull;
use super::super::pipelines::MeshletCullPipelines;

impl MeshletCull {
    /// 2-pass cull with group-atomic LOD descent (#465). Pass 1 atomicMaxes pixel error per
    /// group_index into `group_max_err`; pass 2 reads it to produce group-coherent descent
    /// decisions.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_scene_pool_atomic(
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
            "scene pool atomic total {} exceeds dispatcher capacity {}",
            total_threads,
            self.capacity,
        );
        debug_assert!(
            pool.mesh_count > 0,
            "dispatch_scene_pool_atomic called with an empty pool",
        );

        let params = self.stage_params(queue, cull_params);
        self.stage_scene(queue, scene_params);
        encoder.clear_buffer(&self.visible_count, 0, None);
        // Reset group_max_err to 0 (= 0.0 f32 via bitcast). Pass 1
        // atomicMaxes only positive pixel errors so 0 is a valid
        // "no contribution yet" floor.
        encoder.clear_buffer(&self.group_max_err, 0, None);
        // Reset reject_reasons so stale values from the previous frame don't leak into the overlay
        // raster pass when `debug_active` is set this frame. 0 = "thread skipped", which the
        // overlay treats as no-op.
        if self.rejects {
            encoder.clear_buffer(&self.reject_reasons, 0, None);
        }
        // Reset the per-stage survivor counters (#454.6) — cull shader atomicAdds per cluster, so
        // stale frame N-1 totals would compound otherwise. 16-byte clear is essentially free.
        encoder.clear_buffer(&self.stage_counters, 0, None);

        let binds = self.binds(pipelines, device, pool, scene);

        let (groups_x, groups_y) = tiled_workgroups(total_threads, 64);

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_lod_compute_group_max_err_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.pipeline_lod_compute_group_max_err);
            pass.set_bind_group(0, &binds.cull, &[params]);
            pass.set_bind_group(1, &binds.pool, &[]);
            pass.set_bind_group(2, &binds.scene, &[]);
            pass.set_bind_group(3, &binds.group_err, &[]);
            pass.set_bind_group(4, &binds.debug, &[]);
            self.dispatch_gated(&mut pass, (groups_x, groups_y));
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_scene_pool_atomic_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.pipeline_cull_scene_pool_atomic);
            pass.set_bind_group(0, &binds.cull, &[params]);
            pass.set_bind_group(1, &binds.pool, &[]);
            pass.set_bind_group(2, &binds.scene, &[]);
            pass.set_bind_group(3, &binds.group_err, &[]);
            pass.set_bind_group(4, &binds.debug, &[]);
            self.dispatch_gated(&mut pass, (groups_x, groups_y));
        }

        self.mirror_count_to_indirect_args(encoder);
    }
}

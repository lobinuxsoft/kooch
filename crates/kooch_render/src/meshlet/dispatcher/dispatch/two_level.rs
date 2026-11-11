use crate::meshlet::cull::CullParams;
use crate::meshlet::pool::GpuGlobalMeshPool;
use crate::meshlet::scene::{MeshletScene, SceneCullParams};
use kooch_core::gpu::tiled_workgroups;

use super::super::MeshletCull;
use super::super::pipelines::MeshletCullPipelines;
use super::super::{
    CHUNK_ARGS_OFFSET, CHUNK_HEADER_WORDS, CULL_CHUNK_MESHLETS, DISPATCH_ARGS_BYTES,
};

/// Chunks the worst case needs: every instance surviving, each expanded at the heaviest mesh in the
/// pool.
pub fn chunks_for(instance_count: u32, meshlets_per_mesh: u32) -> u32 {
    instance_count
        .saturating_mul(meshlets_per_mesh.div_ceil(CULL_CHUNK_MESHLETS).max(1))
        .max(1)
}

impl MeshletCull {
    /// The 2-pass atomic cull (#465), entered per instance rather than per rectangle cell (#1002).
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_scene_pool_atomic_chunked(
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
        debug_assert!(
            pool.mesh_count > 0,
            "dispatch_scene_pool_atomic_chunked called with an empty pool",
        );
        debug_assert!(
            scene_params.chunk_capacity <= self.chunk_capacity,
            "scene_params says {} chunks, the buffer holds {}",
            scene_params.chunk_capacity,
            self.chunk_capacity,
        );

        let params = self.stage_params(queue, cull_params);
        self.stage_scene(queue, scene_params);
        encoder.clear_buffer(&self.visible_count, 0, None);
        encoder.clear_buffer(&self.group_max_err, 0, None);
        if self.rejects {
            encoder.clear_buffer(&self.reject_reasons, 0, None);
        }
        encoder.clear_buffer(&self.stage_counters, 0, None);
        // 🔴 The chunk HEADER only.
        encoder.clear_buffer(&self.chunks, 0, Some(CHUNK_HEADER_WORDS * 4));

        let binds = self.binds(pipelines, device, pool, scene);

        let bind = |pass: &mut wgpu::ComputePass<'_>| {
            pass.set_bind_group(0, &binds.cull, &[params]);
            pass.set_bind_group(1, &binds.pool, &[]);
            pass.set_bind_group(2, &binds.scene, &[]);
            pass.set_bind_group(3, &binds.chunked, &[]);
            pass.set_bind_group(4, &binds.debug, &[]);
        };

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_instances_pass"),
                timestamp_writes: None,
            });
            bind(&mut pass);

            pass.set_pipeline(&pipelines.pipeline_cull_instances);
            // Gated, the instance pass alone is enough: the chunk header was cleared above, so a
            // skipped pass expands to zero chunks.
            self.dispatch_gated(&mut pass, tiled_workgroups(scene_params.instance_count, 64));

            pass.set_pipeline(&pipelines.pipeline_cull_expand_args);
            pass.dispatch_workgroups(1, 1, 1);
        }

        encoder.copy_buffer_to_buffer(
            &self.chunks,
            CHUNK_ARGS_OFFSET,
            &self.chunk_args,
            0,
            DISPATCH_ARGS_BYTES,
        );

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_expand_pass"),
                timestamp_writes: None,
            });
            bind(&mut pass);

            // The size of these two was decided by the GPU one pass ago and the CPU never learns
            // it. That is the property the whole change turns on: a CPU-side count would need a
            // readback, and a readback in the hot path is a frame of latency.
            pass.set_pipeline(&pipelines.pipeline_lod_group_max_err_chunked);
            pass.dispatch_workgroups_indirect(&self.chunk_args, 0);

            pass.set_pipeline(&pipelines.pipeline_cull_scene_pool_atomic_chunked);
            pass.dispatch_workgroups_indirect(&self.chunk_args, 0);
        }

        self.mirror_count_to_indirect_args(encoder);
    }
}

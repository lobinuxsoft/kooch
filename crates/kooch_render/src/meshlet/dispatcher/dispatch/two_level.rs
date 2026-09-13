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

        let params_binding = self.stage_params(queue, cull_params);
        queue.write_buffer(
            &self.scene_params_buffer,
            0,
            bytemuck::bytes_of(scene_params),
        );
        encoder.clear_buffer(&self.visible_count, 0, None);
        encoder.clear_buffer(&self.group_max_err, 0, None);
        if self.rejects {
            encoder.clear_buffer(&self.reject_reasons, 0, None);
        }
        encoder.clear_buffer(&self.stage_counters, 0, None);
        // 🔴 The chunk HEADER only.
        encoder.clear_buffer(&self.chunks, 0, Some(CHUNK_HEADER_WORDS * 4));

        let cull_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_chunked_cull_bg"),
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
        let pool_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_chunked_pool_bg"),
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
            label: Some("meshlet_cull_chunked_scene_bg"),
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
        let chunked_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_chunked_bg"),
            layout: &pipelines.chunked_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.group_max_err.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pool.mesh_bounds.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.chunks.as_entire_binding(),
                },
            ],
        });
        let debug_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_chunked_debug_bg"),
            layout: &pipelines.debug_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.reject_reasons.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.stage_counters.as_entire_binding(),
                },
            ],
        });

        let bind = |pass: &mut wgpu::ComputePass<'_>| {
            pass.set_bind_group(0, &cull_bg, &[]);
            pass.set_bind_group(1, &pool_bg, &[]);
            pass.set_bind_group(2, &scene_bg, &[]);
            pass.set_bind_group(3, &chunked_bg, &[]);
            pass.set_bind_group(4, &debug_bg, &[]);
        };

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_instances_pass"),
                timestamp_writes: None,
            });
            bind(&mut pass);

            pass.set_pipeline(&pipelines.pipeline_cull_instances);
            let (groups_x, groups_y) = tiled_workgroups(scene_params.instance_count, 64);
            pass.dispatch_workgroups(groups_x, groups_y, 1);

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

use crate::meshlet::cull::CullParams;
use crate::meshlet::pool::GpuGlobalMeshPool;
use crate::meshlet::scene::{MeshletScene, SceneCullParams};
use kooch_core::gpu::tiled_workgroups;

use super::super::MeshletCull;
use super::super::pipelines::MeshletCullPipelines;
use super::super::{
    CHUNK_ARGS_OFFSET, CHUNK_HEADER_WORDS, CULL_CHUNK_MESHLETS, DISPATCH_ARGS_BYTES,
};

/// Chunks the worst case needs: every instance surviving, each expanded
/// at the heaviest mesh in the pool.
///
/// This is the old rectangle divided by the workgroup size, and it is
/// deliberately still an over-approximation — it sizes a BUFFER, not a
/// dispatch. Four bytes a chunk against nine million lanes is the whole
/// point of #1002.
pub fn chunks_for(instance_count: u32, meshlets_per_mesh: u32) -> u32 {
    instance_count
        .saturating_mul(meshlets_per_mesh.div_ceil(CULL_CHUNK_MESHLETS).max(1))
        .max(1)
}

impl MeshletCull {
    /// The 2-pass atomic cull (#465), entered per instance rather than
    /// per rectangle cell (#1002).
    ///
    /// Four dispatches where [`Self::dispatch_scene_pool_atomic`] had
    /// two:
    ///
    /// 1. `cs_cull_instances` — one thread per instance. Frustum, then
    ///    screen coverage. A survivor reserves
    ///    `⌈its own meshlet_count / 64⌉` chunks.
    /// 2. `cs_cull_expand_args` — one thread turning that count into
    ///    dispatch args.
    /// 3. `cs_lod_group_max_err_chunked` — #465's pass 1, indirect.
    /// 4. `cs_cull_scene_pool_atomic_chunked` — #465's pass 2,
    ///    indirect.
    ///
    /// 🔴 In TWO compute passes with a buffer copy between them, and
    /// that is a wgpu rule rather than a preference: a buffer may not
    /// be `STORAGE_READ_WRITE` and `INDIRECT` inside one usage scope.
    /// `chunks` has to stay bound as storage for the expansion to read
    /// the list, so the three words it dispatches off are copied into
    /// `chunk_args` first — the same move
    /// `mirror_count_to_indirect_args` makes for `visible_count`.
    ///
    /// The two extra dispatches are the price. What they buy on
    /// `dense.scene` is the meshlet domain entered on the order of the
    /// scene's real meshlet count instead of 9 633 630 times, and — the
    /// part that matters more — registering a heavy mesh no longer
    /// changes what a field of cubes costs.
    ///
    /// # Capacity
    ///
    /// `capacity` and `group_capacity` are unchanged: they size the
    /// SURVIVORS and the error arena, neither of which this reshapes.
    /// `chunk_capacity` must cover [`chunks_for`] — call
    /// [`Self::ensure_chunk_capacity`] first.
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
        // 🔴 The chunk HEADER only. Clearing the list too would be a
        // memset of the whole rectangle every frame — the cost this
        // pass exists to stop paying — and it is dead weight: a slot
        // past `chunk_count` is never read, and one below it was
        // written this frame by the instance pass.
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

            // The size of these two was decided by the GPU one pass ago
            // and the CPU never learns it. That is the property the
            // whole change turns on: a CPU-side count would need a
            // readback, and a readback in the hot path is a frame of
            // latency.
            pass.set_pipeline(&pipelines.pipeline_lod_group_max_err_chunked);
            pass.dispatch_workgroups_indirect(&self.chunk_args, 0);

            pass.set_pipeline(&pipelines.pipeline_cull_scene_pool_atomic_chunked);
            pass.dispatch_workgroups_indirect(&self.chunk_args, 0);
        }

        self.mirror_count_to_indirect_args(encoder);
    }
}

use crate::meshlet::cull::CullParams;
use crate::meshlet::pool::GpuGlobalMeshPool;
use crate::meshlet::scene::{MeshletScene, SceneCullParams};

use super::super::MeshletCull;
use super::super::pipelines::MeshletCullPipelines;

impl MeshletCull {
    /// 2-pass cull with group-atomic LOD descent (#465). Pass 1
    /// atomicMaxes pixel error per group_index into `group_max_err`;
    /// pass 2 reads it to produce group-coherent descent decisions.
    /// Sibling meshlets sharing a group always either all descend or
    /// all stay together — no torn coverage seam between LOD levels.
    ///
    /// Caller must call [`Self::ensure_capacity`] +
    /// [`Self::ensure_group_capacity`] beforehand. Visible meshlet
    /// packing matches [`Self::dispatch_scene_pool`]:
    /// `(instance_id << 16) | global_meshlet_idx`.
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

        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(cull_params));
        queue.write_buffer(
            &self.scene_params_buffer,
            0,
            bytemuck::bytes_of(scene_params),
        );
        encoder.clear_buffer(&self.visible_count, 0, None);
        // Reset group_max_err to 0 (= 0.0 f32 via bitcast). Pass 1
        // atomicMaxes only positive pixel errors so 0 is a valid
        // "no contribution yet" floor.
        encoder.clear_buffer(&self.group_max_err, 0, None);
        // Reset reject_reasons so stale values from the previous
        // frame don't leak into the overlay raster pass when
        // `debug_active` is set this frame. 0 = "thread skipped",
        // which the overlay treats as no-op. Cost is negligible
        // (single buffer clear per frame) and only paid when the
        // dispatcher runs the atomic path at all.
        encoder.clear_buffer(&self.reject_reasons, 0, None);
        // Reset the per-stage survivor counters (#454.6) — cull
        // shader atomicAdds per cluster, so stale frame N-1 totals
        // would compound otherwise. 16-byte clear is essentially
        // free.
        encoder.clear_buffer(&self.stage_counters, 0, None);

        // Bind groups shared by both passes.
        let cull_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_scene_pool_atomic_cull_bg"),
            layout: &pipelines.cull_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params_buffer.as_entire_binding(),
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
            label: Some("meshlet_cull_scene_pool_atomic_pool_bg"),
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
            label: Some("meshlet_cull_scene_pool_atomic_scene_bg"),
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
        let group_err_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_group_err_bg"),
            layout: &pipelines.group_err_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.group_max_err.as_entire_binding(),
            }],
        });
        // Group(4) cull debug buffers (#454.4 + #454.6). Bound for
        // both passes because the pipeline layout is shared; the
        // lod-compute entry never references either global so the
        // binding is a no-op there beyond the table write.
        let debug_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_debug_bg"),
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

        let workgroups = total_threads.div_ceil(64).max(1);

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_lod_compute_group_max_err_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.pipeline_lod_compute_group_max_err);
            pass.set_bind_group(0, &cull_bg, &[]);
            pass.set_bind_group(1, &pool_bg, &[]);
            pass.set_bind_group(2, &scene_bg, &[]);
            pass.set_bind_group(3, &group_err_bg, &[]);
            pass.set_bind_group(4, &debug_bg, &[]);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_scene_pool_atomic_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.pipeline_cull_scene_pool_atomic);
            pass.set_bind_group(0, &cull_bg, &[]);
            pass.set_bind_group(1, &pool_bg, &[]);
            pass.set_bind_group(2, &scene_bg, &[]);
            pass.set_bind_group(3, &group_err_bg, &[]);
            pass.set_bind_group(4, &debug_bg, &[]);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        self.mirror_count_to_indirect_args(encoder);
    }
}

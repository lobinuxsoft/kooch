use crate::meshlet::cull::CullParams;
use crate::meshlet::pool::GpuGlobalMeshPool;
use crate::meshlet::scene::{MeshletScene, SceneCullParams};

use super::super::MeshletCull;
use super::super::pipelines::MeshletCullPipelines;
use super::super::types::HiZTestParams;

impl MeshletCull {
    /// Hi-Z 2-pass cull (#445), pass A. Same as
    /// [`Self::dispatch_scene_pool_atomic`] but additionally tests
    /// every meshlet that survives frustum + cone against the
    /// previous frame's Hi-Z pyramid (`hi_z_view`); rejects are
    /// appended to `culled_meshlets` for the pass B retest the
    /// orchestrator dispatches once `hi_z_curr` is rebuilt.
    ///
    /// Caller must call [`Self::ensure_capacity`] +
    /// [`Self::ensure_group_capacity`] beforehand. `hi_z_view` must
    /// be the multi-mip view of a [`crate::hi_z::HiZ`] sized to the
    /// active depth attachment — the view is sampled, not written.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_scene_pool_atomic_hi_z(
        &self,
        pipelines: &MeshletCullPipelines,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &GpuGlobalMeshPool,
        scene: &MeshletScene,
        cull_params: &CullParams,
        scene_params: &SceneCullParams,
        hi_z_params: &HiZTestParams,
        hi_z_view: &wgpu::TextureView,
        arena: &mut Vec<wgpu::BindGroup>,
    ) {
        let total_threads = scene_params.instance_count * scene_params.meshlets_per_mesh;
        debug_assert!(
            total_threads <= self.capacity,
            "scene pool atomic hi-z total {} exceeds dispatcher capacity {}",
            total_threads,
            self.capacity,
        );
        debug_assert!(
            pool.mesh_count > 0,
            "dispatch_scene_pool_atomic_hi_z called with an empty pool",
        );

        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(cull_params));
        queue.write_buffer(
            &self.scene_params_buffer,
            0,
            bytemuck::bytes_of(scene_params),
        );
        queue.write_buffer(&self.hi_z_params_buffer, 0, bytemuck::bytes_of(hi_z_params));
        encoder.clear_buffer(&self.visible_count, 0, None);
        encoder.clear_buffer(&self.culled_count, 0, None);
        encoder.clear_buffer(&self.group_max_err, 0, None);

        let extended_cull_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_scene_pool_atomic_hi_z_extended_cull_bg"),
            layout: &pipelines.extended_cull_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.visible_meshlets.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.visible_count.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.culled_meshlets.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.culled_count.as_entire_binding(),
                },
            ],
        });
        let pool_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_scene_pool_atomic_hi_z_pool_bg"),
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
        let scene_with_hi_z_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_scene_pool_atomic_hi_z_scene_bg"),
            layout: &pipelines.scene_with_hi_z_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: scene.instance_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.scene_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.hi_z_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(hi_z_view),
                },
            ],
        });
        let group_err_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_scene_pool_atomic_hi_z_group_err_bg"),
            layout: &pipelines.group_err_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.group_max_err.as_entire_binding(),
            }],
        });

        let workgroups = total_threads.div_ceil(64).max(1);

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_lod_compute_group_max_err_hi_z_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.pipeline_lod_compute_group_max_err_hi_z);
            pass.set_bind_group(0, &extended_cull_bg, &[]);
            pass.set_bind_group(1, &pool_bg, &[]);
            pass.set_bind_group(2, &scene_with_hi_z_bg, &[]);
            pass.set_bind_group(3, &group_err_bg, &[]);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_scene_pool_atomic_hi_z_pass_a"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.pipeline_cull_scene_pool_atomic_hi_z);
            pass.set_bind_group(0, &extended_cull_bg, &[]);
            pass.set_bind_group(1, &pool_bg, &[]);
            pass.set_bind_group(2, &scene_with_hi_z_bg, &[]);
            pass.set_bind_group(3, &group_err_bg, &[]);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        // Mirror visible_count → indirect_args.instance_count so the
        // pass-A raster has a draw count covering exactly pass A's
        // survivors. Pass B re-mirrors at the end with the union
        // total, and the pass-B raster (LoadOp::Load on the vbuf +
        // depth) re-rasterises pass A's set as well — the depth test
        // makes that idempotent for correctness, and the overhead is
        // bounded by `count_a` extra fragment-shader invocations.
        self.mirror_count_to_indirect_args(encoder);

        // Park the bind groups in the caller's arena so they outlive
        // the encoder's submit. wgpu does not internally Arc-clone
        // bind groups on `set_bind_group`; dropping them between
        // record and submit makes the bound views "invalid" on Mesa
        // radv. Caller clears the arena after the queue.submit cycle
        // completes for this frame.
        arena.push(extended_cull_bg);
        arena.push(pool_bg);
        arena.push(scene_with_hi_z_bg);
        arena.push(group_err_bg);
    }

    /// Hi-Z 2-pass cull (#445), pass B. Drains
    /// `culled_meshlets[0..culled_count]` (pass A's reject queue) and
    /// re-tests every entry against `hi_z_view`, which the
    /// orchestrator points at the *current* frame's pyramid (rebuilt
    /// from the pass-A raster's depth between passes). Survivors
    /// append to `visible_meshlets`; final mirror to `indirect_args`
    /// happens here so a single buffer-to-buffer copy covers both
    /// passes' contributions.
    ///
    /// Bind groups: same `extended_cull` + `scene_with_hi_z` shapes
    /// as pass A — only the pyramid view changes between bind-group
    /// constructions.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_cull_pass_b(
        &self,
        pipelines: &MeshletCullPipelines,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &GpuGlobalMeshPool,
        scene: &MeshletScene,
        hi_z_params: &HiZTestParams,
        hi_z_view: &wgpu::TextureView,
        arena: &mut Vec<wgpu::BindGroup>,
    ) {
        // hi_z_params for pass B targets the freshly-built pyramid;
        // the view_proj / pyramid dims may differ from pass A only
        // when mip count or viewport dims diverged (they shouldn't,
        // but the API takes the params explicitly to keep the call
        // site self-documenting).
        queue.write_buffer(&self.hi_z_params_buffer, 0, bytemuck::bytes_of(hi_z_params));

        let extended_cull_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_pass_b_extended_cull_bg"),
            layout: &pipelines.extended_cull_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.visible_meshlets.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.visible_count.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.culled_meshlets.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.culled_count.as_entire_binding(),
                },
            ],
        });
        let pool_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_pass_b_pool_bg"),
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
        let scene_with_hi_z_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_pass_b_scene_bg"),
            layout: &pipelines.scene_with_hi_z_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: scene.instance_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.scene_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.hi_z_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(hi_z_view),
                },
            ],
        });
        let group_err_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_pass_b_group_err_bg"),
            layout: &pipelines.group_err_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.group_max_err.as_entire_binding(),
            }],
        });

        // Worst-case dispatch: one thread per `capacity` slot. The
        // shader early-outs against `culled_count` so threads past
        // the actual reject-queue length pay only an atomic load.
        let workgroups = self.capacity.div_ceil(64).max(1);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_pass_b_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.pipeline_cull_pass_b);
            pass.set_bind_group(0, &extended_cull_bg, &[]);
            pass.set_bind_group(1, &pool_bg, &[]);
            pass.set_bind_group(2, &scene_with_hi_z_bg, &[]);
            pass.set_bind_group(3, &group_err_bg, &[]);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        // Final mirror: visible_count now reflects pass A + pass B
        // contributions, so the indirect draw picks up the full set.
        self.mirror_count_to_indirect_args(encoder);

        arena.push(extended_cull_bg);
        arena.push(pool_bg);
        arena.push(scene_with_hi_z_bg);
        arena.push(group_err_bg);
    }
}

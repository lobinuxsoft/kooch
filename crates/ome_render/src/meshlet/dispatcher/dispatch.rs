//! `MeshletCull::dispatch` and `dispatch_with_hi_z` — per-frame compute
//! recordings.

use crate::meshlet::cull::CullParams;
use crate::meshlet::gpu_meshlet::GpuMeshletMesh;
use crate::meshlet::pool::GpuGlobalMeshPool;
use crate::meshlet::scene::{MeshletScene, SceneCullParams};

use super::types::HiZTestParams;
use super::MeshletCull;

impl MeshletCull {
    /// Dispatches the cull pass for `mesh` against `params`. Resets
    /// `visible_count` to zero before dispatch so each frame starts
    /// from a clean slate.
    ///
    /// The caller must keep `mesh` alive for the duration of the
    /// encoder submission — bind groups borrow its descriptor buffer.
    pub fn dispatch(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        mesh: &GpuMeshletMesh,
        params: &CullParams,
    ) {
        debug_assert!(
            mesh.meshlet_count <= self.capacity,
            "meshlet count {} exceeds dispatcher capacity {}",
            mesh.meshlet_count,
            self.capacity,
        );

        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(params));
        encoder.clear_buffer(&self.visible_count, 0, None);

        let cull_bg = self.build_cull_bg(device, mesh);

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &cull_bg, &[]);
            let workgroups = mesh.meshlet_count.div_ceil(64);
            pass.dispatch_workgroups(workgroups.max(1), 1, 1);
        }

        self.mirror_count_to_indirect_args(encoder);
    }

    /// Same as [`Self::dispatch`] but runs the Hi-Z-aware cull entry
    /// point. `hi_z_view` must reference the multi-mip pyramid view
    /// produced by [`crate::hi_z::HiZ::full_view`].
    pub fn dispatch_with_hi_z(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        mesh: &GpuMeshletMesh,
        params: &CullParams,
        hi_z_params: &HiZTestParams,
        hi_z_view: &wgpu::TextureView,
    ) {
        debug_assert!(
            mesh.meshlet_count <= self.capacity,
            "meshlet count {} exceeds dispatcher capacity {}",
            mesh.meshlet_count,
            self.capacity,
        );

        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(params));
        queue.write_buffer(&self.hi_z_params_buffer, 0, bytemuck::bytes_of(hi_z_params));
        encoder.clear_buffer(&self.visible_count, 0, None);

        let cull_bg = self.build_cull_bg(device, mesh);
        let hi_z_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_hi_z_bg"),
            layout: &self.hi_z_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.hi_z_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(hi_z_view),
                },
            ],
        });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_hi_z_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline_hi_z);
            pass.set_bind_group(0, &cull_bg, &[]);
            pass.set_bind_group(1, &hi_z_bg, &[]);
            let workgroups = mesh.meshlet_count.div_ceil(64);
            pass.dispatch_workgroups(workgroups.max(1), 1, 1);
        }

        self.mirror_count_to_indirect_args(encoder);
    }

    fn build_cull_bg(
        &self,
        device: &wgpu::Device,
        mesh: &GpuMeshletMesh,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_bg_dispatch"),
            layout: &self.cull_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: mesh.descriptors.as_entire_binding(),
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
        })
    }

    /// Mirror the atomic visible counter into the indirect args'
    /// `instance_count` slot. Offset 4 inside DrawIndirectArgs:
    ///   [0..4)   vertex_count    (constant, set at construction)
    ///   [4..8)   instance_count  (this copy)
    ///   [8..16)  first_vertex / first_instance (zero, immutable)
    fn mirror_count_to_indirect_args(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_buffer_to_buffer(
            &self.visible_count,
            0,
            &self.indirect_args,
            4,
            4,
        );
    }

    /// Scene-wide cull against a SINGLE [`GpuMeshletMesh`]
    /// (Phase 1.E.1). Drives `cs_cull_scene` directly without going
    /// through the [`GpuGlobalMeshPool`]; production code uses
    /// [`Self::dispatch_scene_pool`] instead, but tests retain this
    /// path to validate the cull shader at low level without
    /// constructing a pool. New callers should prefer the pool path.
    ///
    /// # Capacity
    ///
    /// `MeshletCull::capacity` must cover the worst-case sum of visible
    /// meshlets (`instance_count * meshlets_per_mesh`). The dispatcher
    /// debug-asserts this on entry.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_scene(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        mesh: &GpuMeshletMesh,
        scene: &MeshletScene,
        cull_params: &CullParams,
        scene_params: &SceneCullParams,
    ) {
        let total_threads = scene_params.instance_count * scene_params.meshlets_per_mesh;
        debug_assert!(
            total_threads <= self.capacity,
            "scene total (instances × meshlets/mesh) {} exceeds dispatcher capacity {}",
            total_threads,
            self.capacity,
        );

        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(cull_params));
        queue.write_buffer(
            &self.scene_params_buffer,
            0,
            bytemuck::bytes_of(scene_params),
        );
        encoder.clear_buffer(&self.visible_count, 0, None);

        let cull_bg = self.build_cull_bg(device, mesh);
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_scene_bg"),
            layout: &self.scene_bgl,
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
                label: Some("meshlet_cull_scene_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline_scene);
            pass.set_bind_group(0, &cull_bg, &[]);
            pass.set_bind_group(2, &scene_bg, &[]);
            let workgroups = total_threads.div_ceil(64).max(1);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        self.mirror_count_to_indirect_args(encoder);
    }

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

        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(cull_params));
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
            layout: &self.cull_bgl,
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
        // Two-entry bind group matching the cull-only pool BGL (the
        // full GpuGlobalMeshPool::bind_group covers the rasterizer's
        // 5-entry layout).
        let pool_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_scene_pool_pool_bg"),
            layout: &self.pool_bgl,
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
            layout: &self.scene_bgl,
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
            pass.set_pipeline(&self.pipeline_scene_pool);
            pass.set_bind_group(0, &cull_bg, &[]);
            pass.set_bind_group(1, &pool_bg, &[]);
            pass.set_bind_group(2, &scene_bg, &[]);
            let workgroups = total_threads.div_ceil(64).max(1);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        self.mirror_count_to_indirect_args(encoder);
    }

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

        // Bind groups shared by both passes.
        let cull_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_scene_pool_atomic_cull_bg"),
            layout: &self.cull_bgl,
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
            layout: &self.pool_bgl,
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
            layout: &self.scene_bgl,
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
            layout: &self.group_err_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.group_max_err.as_entire_binding(),
            }],
        });

        let workgroups = total_threads.div_ceil(64).max(1);

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_lod_compute_group_max_err_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline_lod_compute_group_max_err);
            pass.set_bind_group(0, &cull_bg, &[]);
            pass.set_bind_group(1, &pool_bg, &[]);
            pass.set_bind_group(2, &scene_bg, &[]);
            pass.set_bind_group(3, &group_err_bg, &[]);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_scene_pool_atomic_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline_cull_scene_pool_atomic);
            pass.set_bind_group(0, &cull_bg, &[]);
            pass.set_bind_group(1, &pool_bg, &[]);
            pass.set_bind_group(2, &scene_bg, &[]);
            pass.set_bind_group(3, &group_err_bg, &[]);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        self.mirror_count_to_indirect_args(encoder);
    }

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
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &GpuGlobalMeshPool,
        scene: &MeshletScene,
        cull_params: &CullParams,
        scene_params: &SceneCullParams,
        hi_z_params: &HiZTestParams,
        hi_z_view: &wgpu::TextureView,
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
            layout: &self.extended_cull_bgl,
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
            layout: &self.pool_bgl,
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
            layout: &self.scene_with_hi_z_bgl,
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
            layout: &self.group_err_bgl,
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
            pass.set_pipeline(&self.pipeline_lod_compute_group_max_err_hi_z);
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
            pass.set_pipeline(&self.pipeline_cull_scene_pool_atomic_hi_z);
            pass.set_bind_group(0, &extended_cull_bg, &[]);
            pass.set_bind_group(1, &pool_bg, &[]);
            pass.set_bind_group(2, &scene_with_hi_z_bg, &[]);
            pass.set_bind_group(3, &group_err_bg, &[]);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        // NOTE: deliberately *not* mirroring visible_count → indirect
        // args here. The orchestrator runs pass B before any raster,
        // so the indirect mirror lives at the end of pass B in
        // `dispatch_cull_pass_b`. Mirroring here would force a tiny
        // copy that pass B's mirror would immediately overwrite.
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
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &GpuGlobalMeshPool,
        scene: &MeshletScene,
        hi_z_params: &HiZTestParams,
        hi_z_view: &wgpu::TextureView,
    ) {
        // hi_z_params for pass B targets the freshly-built pyramid;
        // the view_proj / pyramid dims may differ from pass A only
        // when mip count or viewport dims diverged (they shouldn't,
        // but the API takes the params explicitly to keep the call
        // site self-documenting).
        queue.write_buffer(&self.hi_z_params_buffer, 0, bytemuck::bytes_of(hi_z_params));

        let extended_cull_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_pass_b_extended_cull_bg"),
            layout: &self.extended_cull_bgl,
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
            layout: &self.pool_bgl,
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
            layout: &self.scene_with_hi_z_bgl,
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
            layout: &self.group_err_bgl,
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
            pass.set_pipeline(&self.pipeline_cull_pass_b);
            pass.set_bind_group(0, &extended_cull_bg, &[]);
            pass.set_bind_group(1, &pool_bg, &[]);
            pass.set_bind_group(2, &scene_with_hi_z_bg, &[]);
            pass.set_bind_group(3, &group_err_bg, &[]);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        // Final mirror: visible_count now reflects pass A + pass B
        // contributions, so the indirect draw picks up the full set.
        self.mirror_count_to_indirect_args(encoder);
    }
}

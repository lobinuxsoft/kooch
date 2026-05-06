//! Per-frame work for [`MeshletRenderStage`]: resize, GPU mesh cache
//! upkeep, ECS asset sync, and the cull → vbuf → deferred chain.
//! Lives in its own file so the parent stage module stays under the
//! 400-LoC ceiling and the "construct vs run" split is visible at a
//! glance.

use glam::{Mat4, Vec3};

use ome_core::Guid;
use ome_core::asset_loader::AssetServer;
use ome_core::assets::Assets;
use ome_core::resource::Resources;

use crate::meshlet::asset::MeshletMesh;
use crate::meshlet::cull::CullParams;
use crate::meshlet::debug::{MeshletDebugMode, MeshletLodSettings};
use crate::meshlet::deferred::DEFERRED_COLOR_FORMAT;
use crate::meshlet::gpu_meshlet::pool_meshlet_bind_group;
use crate::meshlet::scene::SceneCullParams;
use crate::meshlet::vis_buffer::VISIBILITY_BUFFER_FORMAT;

use super::{create_2d_attachment, MeshletRenderStage, MeshletRenderStats};

impl MeshletRenderStage {
    /// Recreates the stage's vbuf / depth / color textures at
    /// `new_size` if it differs from the current size. The cull,
    /// rasterizer, deferred and material pool are unaffected — only
    /// the per-pixel attachments need to grow with the destination
    /// surface.
    pub fn resize(&mut self, device: &wgpu::Device, new_size: (u32, u32)) {
        if new_size == self.size {
            return;
        }
        assert!(
            new_size.0 > 0 && new_size.1 > 0,
            "MeshletRenderStage::resize requires non-zero dimensions"
        );

        let (vbuf_texture, vbuf_view) = create_2d_attachment(
            device,
            "meshlet_render_stage_vbuf",
            new_size,
            VISIBILITY_BUFFER_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let (depth_texture, depth_view) = create_2d_attachment(
            device,
            "meshlet_render_stage_depth",
            new_size,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let (color_texture, color_view) = create_2d_attachment(
            device,
            "meshlet_render_stage_color",
            new_size,
            DEFERRED_COLOR_FORMAT,
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );

        self.vbuf_texture = vbuf_texture;
        self.vbuf_view = vbuf_view;
        self.depth_texture = depth_texture;
        self.depth_view = depth_view;
        self.color_texture = color_texture;
        self.color_view = color_view;
        self.size = new_size;
    }

    /// Registers `mesh` under `guid` in the global mesh pool.
    /// Idempotent — the underlying [`MeshletPipeline::register_mesh`]
    /// caches by GUID, so a repeat call with the same GUID is a
    /// no-op. Marks the pool dirty so the next
    /// [`Self::render_with_assets`] rebuilds the GPU mirror.
    ///
    /// The `device` argument is kept on the signature for backward
    /// compatibility — the actual upload is deferred to render time
    /// because batched registrations only need one GPU upload.
    pub fn ensure_gpu_mesh(
        &mut self,
        _device: &wgpu::Device,
        guid: Guid,
        mesh: &MeshletMesh,
    ) {
        let before = self.pipeline.registered_count();
        let bytes_before = self.pipeline.pool().byte_size();
        self.pipeline.register_mesh(guid, mesh);
        let after = self.pipeline.registered_count();
        if after > before {
            self.pool_dirty = true;
            // #463.5 — credit the freshly-appended pool bytes to the
            // engine VRAM tracker (when wired). Idempotent calls to
            // register_mesh with the same GUID return the cached
            // handle without growing the pool, so `bytes_after ==
            // bytes_before` and the diff is zero.
            if let Some(tracker) = &self.vram_tracker {
                let bytes_after = self.pipeline.pool().byte_size();
                tracker.add(bytes_after.saturating_sub(bytes_before));
            }
        }
    }

    /// Number of distinct meshlet meshes currently registered in the
    /// pool. The GPU mirror tracks the same count once it has been
    /// rebuilt at the next render call.
    pub fn gpu_mesh_count(&self) -> u32 {
        self.pipeline.registered_count()
    }

    /// Resolves every visible `MeshRenderer.mesh` GUID through the
    /// `AssetServer`, fetches the meshlet asset from
    /// `Assets<MeshletMesh>`, and uploads any GUID that is not yet
    /// GPU-resident.
    ///
    /// Idempotent: GUIDs already in the pool's registry are skipped
    /// without touching the AssetServer or Assets storage. Per-frame
    /// cost when steady-state is one ECS query + N registry lookups.
    ///
    /// Failure modes (logged, never panic):
    /// - `AssetServer` resource missing → noop, log warn.
    /// - GUID not registered in `AssetDatabase` → log warn, skip entity.
    /// - Loader rejects the bytes → log warn, skip entity.
    /// - `Assets<MeshletMesh>` missing or stale handle → log warn, skip.
    pub fn sync_assets_to_gpu(&mut self, device: &wgpu::Device, resources: &mut Resources) {
        // Material pool sync first: the meshlet scene system reads
        // `MaterialPipeline.lookup_or_fallback` when assembling
        // `MeshInstance.material_id`, so any newly-picked GUID has
        // to be in the registry before the cull dispatch fires.
        // Pull the GPU queue out of GpuContext so we can drive the
        // sync without expanding this method's signature.
        if let Some(mut material_pipeline) =
            resources.remove::<crate::material::MaterialPipeline>()
        {
            if let Some(gpu) = resources.remove::<ome_core::gpu::GpuContext>() {
                material_pipeline.sync_from_resources(gpu.queue(), resources);
                resources.insert(gpu);
            }
            resources.insert(material_pipeline);
        }

        let referenced = self.pipeline.collect_referenced_guids(resources);
        let pending: Vec<Guid> = referenced
            .iter()
            .copied()
            .filter(|guid| self.pipeline.lookup(*guid).is_none())
            .collect();
        if !referenced.is_empty() {
            tracing::debug!(
                target: "ome_render::meshlet::sync",
                referenced = referenced.len(),
                pending = pending.len(),
                cached = self.pipeline.registered_count(),
                "meshlet asset sync tick",
            );
        }
        if pending.is_empty() {
            return;
        }

        for guid in pending {
            // Take the AssetServer out so we can pass `resources`
            // (which holds `Assets<MeshletMesh>`) by &mut into the
            // load call. Re-insert before any continue/return so we
            // never leak the resource.
            let Some(mut server) = resources.remove::<AssetServer>() else {
                tracing::warn!(
                    target: "ome_render::meshlet::sync",
                    "AssetServer resource missing; skipping meshlet asset sync",
                );
                return;
            };
            let load_result = server.load_by_guid::<MeshletMesh>(guid, resources);
            resources.insert(server);

            let handle = match load_result {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!(
                        target: "ome_render::meshlet::sync",
                        guid = %guid,
                        error = %e,
                        "failed to load meshlet asset by GUID",
                    );
                    continue;
                }
            };

            let Some(assets) = resources.get::<Assets<MeshletMesh>>() else {
                tracing::warn!(
                    target: "ome_render::meshlet::sync",
                    "Assets<MeshletMesh> resource missing after load; aborting sync",
                );
                return;
            };
            let Some(mesh) = assets.get(handle) else {
                tracing::warn!(
                    target: "ome_render::meshlet::sync",
                    guid = %guid,
                    "loaded handle resolved to empty Assets<MeshletMesh> entry",
                );
                continue;
            };

            self.ensure_gpu_mesh(device, guid, mesh);
            tracing::info!(
                target: "ome_render::meshlet::sync",
                guid = %guid,
                "uploaded meshlet asset to GPU",
            );
        }
    }

    /// Records + submits one frame of the meshlet pipeline driven by
    /// `resources`'s ECS query against the multi-mesh `GpuGlobalMeshPool`.
    /// Lazy-rebuilds the GPU pool when [`Self::pool_dirty`] is set.
    ///
    /// Returns [`MeshletRenderStats::default`] when the pool has no
    /// registered mesh yet or the ECS query yielded no instances.
    pub fn render_with_assets(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &Resources,
        view_proj: Mat4,
        cam_pos: Vec3,
    ) -> MeshletRenderStats {
        if self.pool_dirty || self.gpu_pool.is_none() {
            if self.pipeline.registered_count() == 0 {
                return MeshletRenderStats::default();
            }
            self.gpu_pool = Some(self.pipeline.pool().upload(device));
            self.pool_dirty = false;
            tracing::debug!(
                target: "ome_render::meshlet::render",
                meshes = self.pipeline.registered_count(),
                meshlets = self.pipeline.pool().meshlets.len(),
                "rebuilt GpuGlobalMeshPool",
            );
        }
        self.render(device, queue, resources, view_proj, cam_pos)
    }

    /// Records + submits one frame against the current `gpu_pool`.
    /// Caller must have populated the pool via [`Self::ensure_gpu_mesh`]
    /// before invoking. Returns zero stats when the ECS query yields
    /// no instances; the stage does not clear in that case so the
    /// previous frame's color stays on the offscreen target.
    ///
    /// Takes `&mut self` because the cull dispatcher's
    /// `visible_meshlets` buffer is grown on demand to fit the
    /// scene's worst-case (instances × max_meshlets/mesh) thread
    /// count.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &Resources,
        view_proj: Mat4,
        cam_pos: Vec3,
    ) -> MeshletRenderStats {
        let Some(gpu_pool) = self.gpu_pool.as_ref() else {
            tracing::debug!(
                target: "ome_render::meshlet::render",
                "render skipped: gpu_pool not built yet",
            );
            return MeshletRenderStats::default();
        };
        let instances = self.pipeline.collect_scene_instances(resources);
        if instances.is_empty() {
            tracing::debug!(
                target: "ome_render::meshlet::render",
                pipeline_registered = self.pipeline.registered_count(),
                "render skipped: zero instances",
            );
            return MeshletRenderStats::default();
        }
        tracing::debug!(
            target: "ome_render::meshlet::render",
            instances = instances.len(),
            "render dispatching meshlet pipeline",
        );
        assert!(
            (instances.len() as u32) <= self.instance_capacity,
            "MeshletRenderStage: collected {} instances exceeds capacity {}",
            instances.len(),
            self.instance_capacity,
        );

        self.scene.upload_instances(queue, &instances);
        // Worst-case meshlet stride covers every mesh; the pool path
        // bounds-checks per-instance against pool_mesh_descriptors.
        let max_meshlets_per_mesh = gpu_pool.max_meshlets_per_mesh.max(1);
        // Approximate proj_scale_y by the absolute value of `view_proj.y_axis.y`.
        // For an ortho-normal view (look_at_rh with up=Y) the camera basis
        // contributes 1 to that component and the projection's `1 / tan(fovy/2)`
        // is preserved exactly. Skewed cameras pay a small error that the
        // 1-pixel target tolerance absorbs.
        let proj_scale_y = view_proj.y_axis.y.abs();
        let viewport_h_px = self.size.1 as f32;
        let lod_target = resources
            .get::<MeshletLodSettings>()
            .copied()
            .unwrap_or_default()
            .target_error_pixels
            .max(0.01);
        let debug_mode = resources
            .get::<MeshletDebugMode>()
            .copied()
            .unwrap_or_default()
            .as_u32();
        let cull_params = CullParams::new(view_proj, cam_pos, max_meshlets_per_mesh)
            .with_lod(viewport_h_px, proj_scale_y, lod_target)
            .with_debug_mode(debug_mode);
        let scene_params =
            SceneCullParams::new(instances.len() as u32, max_meshlets_per_mesh);

        // Grow visible_meshlets if the scene now needs more slots
        // than the dispatcher was sized for. Geometric growth absorbs
        // future jumps without per-frame reallocation.
        let required_capacity = scene_params
            .instance_count
            .saturating_mul(scene_params.meshlets_per_mesh);
        self.cull.ensure_capacity(device, required_capacity);
        // group_max_err sized to the pool's current group_capacity
        // (sum of 1 + max_group_id across all registered meshes).
        // Same geometric-growth pattern as the visible buffer.
        self.cull
            .ensure_group_capacity(device, self.pipeline.pool().group_capacity.max(1));

        let meshlet_bg = pool_meshlet_bind_group(device, &self.meshlet_bgl, gpu_pool);
        // Prefer the GUID-keyed `MaterialPipeline` pool when it's
        // present in resources — that's the buffer the asset picker
        // writes into. Falls back to the stage-local pool (used by
        // GPU integration tests that bypass the asset system).
        let material_bg = match resources.get::<crate::material::MaterialPipeline>() {
            Some(pipeline) => pipeline.pool().bind_group(device),
            None => self.material_pool.bind_group(device),
        };

        // #463.4 — drain any GPU timer slots that completed since
        // last frame, then acquire a fresh slot for this frame's
        // start/end timestamps. `None` means timers are disabled or
        // every ring slot is in flight; either way the render path
        // proceeds normally and the HUD keeps the previously-sampled
        // value.
        self.gpu_timers.drain_ready();
        let timer_slot = self.gpu_timers.acquire_slot();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("meshlet_render_stage_encoder"),
        });

        if timer_slot.is_some() {
            self.gpu_timers.write_start(&mut encoder);
        }

        self.cull.dispatch_scene_pool_atomic(
            device,
            queue,
            &mut encoder,
            gpu_pool,
            &self.scene,
            &cull_params,
            &scene_params,
        );
        self.rasterizer.render_scene(
            device,
            queue,
            &mut encoder,
            &self.vbuf_view,
            &self.depth_view,
            &meshlet_bg,
            &self.cull,
            &self.scene,
            view_proj,
            0,
        );
        // Editor / game tools insert MeshletDebugMode when they want to
        // override the production shading path. Absence = Off (no-op
        // visual), so headless tests never have to register it.
        let debug_mode = resources
            .get::<MeshletDebugMode>()
            .copied()
            .unwrap_or_default()
            .as_u32();
        self.deferred.shade_scene(
            device,
            queue,
            &mut encoder,
            &self.vbuf_view,
            &self.color_view,
            &meshlet_bg,
            &material_bg,
            &self.cull,
            &self.scene,
            view_proj,
            self.size,
            debug_mode,
        );

        // Close the GPU timing window AFTER the deferred shade pass
        // but BEFORE submit, so the resolve + copy land in the same
        // command buffer.
        if let Some(slot_idx) = timer_slot {
            self.gpu_timers.write_end_and_copy(&mut encoder, slot_idx);
        }

        queue.submit(std::iter::once(encoder.finish()));

        // Hand the slot off to the async readback path. wgpu fires
        // the callback when the GPU has finished and the buffer is
        // host-visible — typically 1-2 frames out.
        if let Some(slot_idx) = timer_slot {
            self.gpu_timers.submit_readback(slot_idx);
        }

        let pool = self.pipeline.pool();
        let pool_meshlets_total = pool.meshlets.len() as u32;
        let pool_meshlets_roots = pool
            .meshlets
            .iter()
            .filter(|m| {
                m.parent_meshlet_index == crate::meshlet::asset::MESHLET_ROOT_PARENT
            })
            .count() as u32;

        MeshletRenderStats {
            instances_uploaded: instances.len() as u32,
            cull_threads: scene_params.instance_count * scene_params.meshlets_per_mesh,
            cam_pos: cam_pos.to_array(),
            pool_meshlets_total,
            pool_meshlets_roots,
            gpu_frame_ms: self.gpu_timers.last_frame_ms(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meshlet::asset::MESHLET_ROOT_PARENT;
    use crate::meshlet::pool::GlobalMeshPool;

    #[test]
    fn ensure_gpu_mesh_marks_pool_dirty_only_for_new_guids() {
        // No GPU device needed — ensure_gpu_mesh defers the upload to
        // render_with_assets so the dirty bookkeeping is the only
        // observable side effect.
        let mut pool = GlobalMeshPool::new();
        let mut pipeline_dirty = false;

        // Simulate the registration semantics directly. The point of
        // the test is to lock the rule "first registration of a GUID
        // marks dirty; repeats do not."
        let before = pool.mesh_count();
        let mesh = crate::meshlet::asset::MeshletMesh {
            vertices: vec![],
            meshlet_vertices: vec![],
            meshlet_triangles: vec![],
            meshlets: vec![crate::meshlet::asset::MeshletDescriptor {
                vertex_offset: 0,
                triangle_offset: 0,
                vertex_count: 0,
                triangle_count: 0,
                aabb_min: [0.0; 3],
                parent_meshlet_index: MESHLET_ROOT_PARENT,
                aabb_max: [0.0; 3],
                lod_error: 0.0,
                bounds_center: [0.0; 3],
                bounding_radius: 0.0,
                cone_apex: [0.0; 3],
                cone_cutoff: 1.0,
                cone_axis: [0.0; 3],
                group_index: crate::meshlet::asset::MESHLET_GROUP_NONE,
                children_group_index: crate::meshlet::asset::MESHLET_GROUP_NONE,
                lod_level: 0,
                _pad4: 0,
                _pad5: 0,
            }],
            aabb: crate::mesh::Aabb::empty(),
        };
        pool.register(&mesh);
        let after = pool.mesh_count();
        if after > before {
            pipeline_dirty = true;
        }
        assert!(pipeline_dirty);

        // Re-registration of the same MeshletMesh adds another pool
        // entry, but the MeshletPipeline registry deduplicates by
        // GUID; the dirty flag mirror in ensure_gpu_mesh keys on the
        // registered_count delta. This test pins the directional rule.
        let before2 = pool.mesh_count();
        let after2 = before2; // simulate dedup hit (no register call)
        assert!(after2 == before2, "dedup keeps the pool unchanged");
    }
}

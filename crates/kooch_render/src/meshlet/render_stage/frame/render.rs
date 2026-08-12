//! `pub fn render_with_assets` + `pub fn render` — the per-frame
//! orchestrator entry points.
//!
//! `render` is a thin dispatcher: it runs the prelude common to both
//! GPU paths (pool sync, ECS query, instance upload, growth checks,
//! per-frame bind groups, GPU timer slot, encoder) and then routes the
//! frame either through [`Self::render_path_r64`] (#493 atomic R64
//! vbuf, sibling file `render_r64.rs`) when the device supports it, or
//! through [`Self::render_path_hi_z_two_pass`] (legacy R32 + Hi-Z
//! 2-pass, sibling file `render_hi_z_2pass.rs`).
//!
//! Both extracted methods own their submits + readbacks and return
//! the [`MeshletRenderStats`] for the frame.

use kooch_core::resource::Resources;

use crate::meshlet::cull::CullParams;
use crate::meshlet::debug::{MeshletDebugMode, MeshletLodSettings};
use crate::meshlet::gpu_meshlet::pool_meshlet_bind_group;
use crate::meshlet::scene::SceneCullParams;
use crate::view_camera::ViewCamera;

use super::super::{MeshletRenderStage, MeshletRenderStats, ViewId};

impl MeshletRenderStage {
    /// Records + submits one frame of the meshlet pipeline driven by
    /// `resources`'s ECS query against the multi-mesh `GpuGlobalMeshPool`.
    /// Lazy-rebuilds the GPU pool when [`Self::pool_dirty`] is set.
    ///
    /// Returns [`MeshletRenderStats::default`] when the pool has no
    /// registered mesh yet or the ECS query yielded no instances.
    pub fn render_with_assets(
        &mut self,
        view_id: ViewId,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &Resources,
        camera: &ViewCamera,
        aspect: f32,
    ) -> MeshletRenderStats {
        // The root of every frame's flamegraph (#785). Named for what a
        // reader is looking for — "the frame" — rather than for the
        // function, because a flamegraph of function names tells nobody
        // which part of the engine to open.
        profiling::scope!("frame");
        if self.pool_dirty || self.gpu_pool.is_none() {
            if self.pipeline.registered_count() == 0 {
                return MeshletRenderStats::default();
            }
            self.gpu_pool = Some(self.pipeline.pool().upload(device));
            self.pool_dirty = false;
            tracing::debug!(
                target: "kooch_render::meshlet::render",
                meshes = self.pipeline.registered_count(),
                meshlets = self.pipeline.pool().meshlets.len(),
                "rebuilt GpuGlobalMeshPool",
            );
        }
        self.render(view_id, device, queue, resources, camera, aspect)
    }

    /// Same as [`Self::render_with_assets`], for this stage's primary
    /// view. Kept because most callers own exactly one.
    pub fn render_with_assets_primary(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &Resources,
        camera: &ViewCamera,
        aspect: f32,
    ) -> MeshletRenderStats {
        self.render_with_assets(self.primary, device, queue, resources, camera, aspect)
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
    ///
    /// # Draw-call accounting (#492)
    ///
    /// `MeshletRenderStats::draw_calls` reports **only the meshlet
    /// stage's contribution**:
    /// - `0` when the ECS query yields no instances (early return below
    ///   skips both the cull dispatch and every raster / deferred
    ///   submit).
    /// - `4` on the atomic R64 vbuf path (#493): cull + clear + raster
    ///   + deferred shade, all single-pass.
    /// - `6` on the legacy R32 + Hi-Z 2-pass path: cull A + raster A +
    ///   SPD pyramid build + cull B + raster B + deferred shade.
    ///
    /// The editor surface adds 3 fixed passes (sky background +
    /// viewport blit + egui paint) outside this stage; the perf-HUD
    /// `Draw calls / frame` field sums both contributions. An empty
    /// scene therefore reports `3`, not `0` — that is the editor base,
    /// not a leak.
    ///
    /// # Visibility filtering (#492)
    ///
    /// `MeshRenderer.visible == false` is filtered upstream at
    /// [`MeshletPipeline::collect_scene_instances`], so an invisible
    /// entity never enters the `Vec<MeshInstance>` and never reaches
    /// the cull dispatch. Same filter runs in
    /// [`MeshletPipeline::collect_referenced_guids`] so an invisible
    /// mesh is also not pulled into the GPU pool. There is no
    /// per-instance visibility flag inside the cull shader because the
    /// upstream filter makes one redundant.
    pub fn render(
        &mut self,
        view_id: ViewId,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &Resources,
        camera: &ViewCamera,
        aspect: f32,
    ) -> MeshletRenderStats {
        // The lens rather than the matrix, because the shadow cascades
        // need the near and far planes to place themselves and a
        // `Mat4` has already thrown them away (#476).
        let view_proj = camera.view_proj(aspect);
        let cam_pos = camera.position();

        // ── Prelude: shared between both GPU paths ─────────────────
        // Extract the per-frame `max_meshlets_per_mesh` immediately so
        // the `&self.gpu_pool` borrow is released before any `&mut
        // self` mutation (cull growth, scene upload). The path methods
        // re-borrow the pool internally for their dispatch calls.
        let max_meshlets_per_mesh = match self.gpu_pool.as_ref() {
            Some(pool) => pool.max_meshlets_per_mesh.max(1),
            None => {
                tracing::debug!(
                    target: "kooch_render::meshlet::render",
                    "render skipped: gpu_pool not built yet",
                );
                return MeshletRenderStats::default();
            }
        };
        let instances = self.pipeline.collect_scene_instances(resources);
        if instances.is_empty() {
            tracing::debug!(
                target: "kooch_render::meshlet::render",
                pipeline_registered = self.pipeline.registered_count(),
                "render skipped: zero instances",
            );
            return MeshletRenderStats::default();
        }
        tracing::debug!(
            target: "kooch_render::meshlet::render",
            instances = instances.len(),
            "render dispatching meshlet pipeline",
        );
        // Grow to fit rather than abort. A scene is authored, not
        // declared: the count arrives from the ECS walk above, and the
        // construction-time capacity was only ever a starting guess.
        // Before this, the 257th instance panicked — in the editor and
        // in a shipped game alike.
        //
        // Ahead of any encoder for this frame, so the buffer being
        // replaced cannot be in flight.
        let required = instances.len() as u32;
        self.scene.ensure_capacity(device, required);
        self.instance_capacity = self.scene.capacity();

        profiling::scope!("upload instances");
        self.scene.upload_instances(queue, &instances);
        // The whole scene in one number, for the point-shadow cube cache
        // (#778). Hashed over the bytes that go to the GPU, so anything
        // that could move a shadow — a transform, a mesh swap, an
        // instance appearing — changes it, and nothing that cannot does.
        // O(n) over a Vec that was just walked to upload it.
        self.scene_hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytemuck::cast_slice::<_, u8>(&instances).hash(&mut hasher);
            hasher.finish()
        };

        let scene_params = SceneCullParams::new(instances.len() as u32, max_meshlets_per_mesh);
        // Worst case for every cull this frame, the view's and the four
        // cascades': one thread per instance-meshlet pair.
        let required_capacity = scene_params
            .instance_count
            .saturating_mul(scene_params.meshlets_per_mesh);
        // group_max_err sized to the per-instance prefix-sum total
        // (Σ over instances of mesh_descriptors[mesh_id].group_count),
        // not the pool's group_capacity. Per-mesh sizing collapsed
        // every instance of the same mesh into one slot range and
        // forced multi-instance LOD descent to the closest one's
        // verdict (#474).
        let required_group_capacity = self.pipeline.instance_group_capacity(&instances).max(1);

        // The sun's cascades (#476). Ahead of the encoder because it can
        // allocate the atlas and grow four culls, and `None` when
        // nothing casts.
        let shadows = self.prepare_shadows(
            device,
            resources,
            camera,
            aspect,
            required_capacity,
            required_group_capacity,
        );
        // Inti's per-frame walk. Ahead of the encoder for the same
        // reason `ensure_capacity` is: growing the light buffer
        // replaces it, and a replaced buffer must not be one an
        // already-recorded pass references.
        self.lights.update(
            device,
            queue,
            resources,
            cam_pos,
            shadows.as_ref().map(|s| s.frame),
        );
        // Worst-case meshlet stride covers every mesh; the pool path
        // bounds-checks per-instance against pool_mesh_descriptors.
        // (`max_meshlets_per_mesh` was bound from `gpu_pool` above so
        // the borrow released before the &mut-self upload.)
        // Orientation-independent: see `projection_scale_y`. Reading a
        // single matrix element here used to disable the LOD selector
        // outright at 90° of roll or looking straight down.
        let proj_scale_y = crate::meshlet::cull::projection_scale_y(view_proj);
        let viewport_h_px = self.views[view_id].size.1 as f32;
        let lod_target = resources
            .get::<MeshletLodSettings>()
            .copied()
            .unwrap_or_default()
            .target_error_pixels
            .max(0.01);
        let debug_mode_enum = resources
            .get::<MeshletDebugMode>()
            .copied()
            .unwrap_or_default();
        let debug_mode = debug_mode_enum.as_u32();
        // Reject-overlay modes need the cull pass to record per-thread
        // reasons into the `reject_reasons[]` SSBO. Production rendering
        // and every non-reject debug mode leaves this off so the cull
        // hot path doesn't pay the conditional store.
        let debug_active = debug_mode_enum.reject_reason_code().is_some();
        let cull_params = CullParams::new(view_proj, cam_pos, max_meshlets_per_mesh)
            .with_lod(viewport_h_px, proj_scale_y, lod_target)
            .with_debug_mode(debug_mode)
            .with_debug_active(debug_active);

        // Grow visible_meshlets if the scene now needs more slots
        // than the dispatcher was sized for. Geometric growth absorbs
        // future jumps without per-frame reallocation.
        self.views[view_id]
            .cull
            .ensure_capacity(device, required_capacity);
        self.views[view_id]
            .cull
            .ensure_group_capacity(device, required_group_capacity);

        // Build the meshlet + material bind groups. The `gpu_pool`
        // re-borrow lives only as long as `meshlet_bg` construction;
        // after that the path methods own &mut self exclusively.
        let meshlet_bg = {
            let gpu_pool = self.gpu_pool.as_ref().expect("checked above");
            pool_meshlet_bind_group(device, &self.meshlet_bgl, gpu_pool)
        };
        // `MaterialPipeline` is the single source of truth for the
        // material pool — see #447. Headless tests must insert one
        // via `Resources::insert(MaterialPipeline::with_capacity(...))`
        // before calling `render_with_assets`.
        let material_bg = resources
            .get::<crate::material::MaterialPipeline>()
            .expect(
                "MaterialPipeline missing in Resources; \
                 insert one before calling render_with_assets",
            )
            .pool()
            .bind_group(device);

        // #463.4 — drain any GPU timer slots that completed since
        // last frame, then acquire a fresh slot for this frame's
        // start/end timestamps. `None` means timers are disabled or
        // every ring slot is in flight; either way the render path
        // proceeds normally and the HUD keeps the previously-sampled
        // value.
        self.gpu_timers.drain_ready();
        let timer_slot = self.gpu_timers.acquire_slot();

        // #454.6 — same pattern for the per-stage survivor counter
        // ring. Drain whatever the wgpu driver thread completed
        // since last frame so `MeshletRenderStats.cull_stage_counts`
        // reports the freshest value the editor stats overlay can
        // surface.
        self.stage_counters.drain_ready();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("meshlet_render_stage_encoder"),
        });

        if timer_slot.is_some() {
            self.gpu_timers.write_start(&mut encoder);
        }

        let instance_count = instances.len() as u32;

        // Contact shadows (#735). Built here rather than in each path
        // because both need it and only this function still holds the
        // camera's lens: `near` and `far` are what turn a stored depth
        // back into metres, and a `Mat4` has thrown them away.
        self.frames_recorded = self.frames_recorded.wrapping_add(1);
        let contact = crate::contact_shadow::ContactShadowUbo::new(
            view_proj,
            camera.near,
            &resources
                .get::<crate::contact_shadow::ContactShadowSettings>()
                .copied()
                .unwrap_or_default(),
            self.frames_recorded,
        );

        // First in the encoder: every shading pass below samples the
        // atlas this fills. Inside the timer, because a shadow pass that
        // costs four culls and four rasters is part of the frame whether
        // or not the HUD says so.
        if let Some(prepared) = shadows.as_ref() {
            // #785 — the shadow passes are four culls and four rasters
            // plus a cube face per point light, and until now their
            // cost was inside whatever number the frame reported.
            let scopes = resources.get::<kooch_core::gpu::GpuScopes>();
            let query = scopes.map(|s| s.begin("shadows", &mut encoder));
            self.record_shadows(
                device,
                queue,
                &mut encoder,
                prepared,
                &meshlet_bg,
                instance_count,
                max_meshlets_per_mesh,
                lod_target,
            );
            if let (Some(scopes), Some(query)) = (scopes, query) {
                scopes.end(&mut encoder, query);
            }
        }

        // ── Path switch ─────────────────────────────────────────────
        // Atomic R64 vbuf path (#493) when the device supports it;
        // otherwise the legacy R32 + Hi-Z 2-pass orchestrator.
        if self.views[view_id].vbuf64_stage.is_some() {
            return self.render_path_r64(
                view_id,
                device,
                queue,
                encoder,
                resources,
                view_proj,
                cam_pos,
                &cull_params,
                &scene_params,
                &meshlet_bg,
                &contact,
                timer_slot,
                instance_count,
            );
        }

        self.render_path_hi_z_two_pass(
            view_id,
            device,
            queue,
            encoder,
            resources,
            view_proj,
            cam_pos,
            &cull_params,
            &scene_params,
            &meshlet_bg,
            &material_bg,
            &contact,
            timer_slot,
            instance_count,
        )
    }
}

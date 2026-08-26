//! Atomic R64 visibility-buffer path (#493) — single-pass cull → R64
//! winner-takes-all raster → deferred shade. Routed by
//! [`MeshletRenderStage::render`] when `vbuf64_stage` is `Some`.
//!
//! Owns the encoder it receives, writes the GPU timer end + readback,
//! submits, and returns the per-frame [`MeshletRenderStats`].

use glam::{Mat4, Vec3};

use kooch_core::resource::Resources;

use crate::meshlet::cull::CullParams;
use crate::meshlet::debug::MeshletDebugMode;
use crate::meshlet::scene::SceneCullParams;

use super::super::{MeshletRenderStage, MeshletRenderStats, ViewId};

impl MeshletRenderStage {
    /// Atomic R64 vbuf path. Same observable contract as the legacy
    /// path: 4 logical passes (cull + clear + raster + shade) and
    /// stats reporting `draw_calls = 4`. See [`Self::render`] for the
    /// dispatch decision and the prelude that builds `cull_params` /
    /// `scene_params` / `meshlet_bg` / `timer_slot`. The material pool
    /// reaches the two-pass shader via the `MaterialPipeline` resource,
    /// so this path takes no precomputed material bind group.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_path_r64(
        &mut self,
        view_id: ViewId,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        resources: &Resources,
        view_proj: Mat4,
        // The camera's own matrix, before the sub-pixel jitter (#481).
        // Equal to `view_proj` when the temporal resolve is off.
        unjittered_view_proj: Mat4,
        cam_pos: Vec3,
        cull_params: &CullParams,
        scene_params: &SceneCullParams,
        meshlet_bg: &wgpu::BindGroup,
        contact: &crate::contact_shadow::ContactShadowUbo,
        timer_slot: Option<usize>,
        instance_count: u32,
    ) -> MeshletRenderStats {
        let gpu_pool = self.gpu_pool.as_ref().expect("checked by render() prelude");

        profiling::scope!("path: R64 atomic vbuf");

        // #785 — the GPU counterpart of the CPU scopes below. Read
        // once: `begin`/`end` take `&self`, so this borrow coexists
        // with everything else this function reads out of `resources`.
        let scopes = resources.get::<kooch_core::gpu::GpuScopes>();

        // #536 — the DLSS handles. Their own resource rather than a
        // field of `GpuContext`, because the frame's systems have taken
        // the context out of `Resources` for the duration of this call.
        let dlss_runtime = resources.get::<kooch_core::gpu::DlssRuntime>();

        // Stage 0 (Cull). render() already called `write_start`
        // which lands on stage 0; just close it after the dispatch.
        {
            profiling::scope!("cull: view");
            let query = scopes.map(|s| s.begin("cull", &mut encoder));
            self.views[view_id].cull.dispatch_scene_pool_atomic(
                &self.cull_pipelines,
                device,
                queue,
                &mut encoder,
                gpu_pool,
                &self.scene,
                cull_params,
                scene_params,
            );
            if let (Some(scopes), Some(query)) = (scopes, query) {
                scopes.end(&mut encoder, query);
            }
        }
        if timer_slot.is_some() {
            self.gpu_timers.write_stage_end(&mut encoder, 0);
        }
        let debug_mode = resources
            .get::<MeshletDebugMode>()
            .copied()
            .unwrap_or_default();
        // #454: the modal accumulator runs in TriangleDensity /
        // Overdraw modes (and falls back to the TriangleDensity
        // count for the reject overlay modes still being wired in
        // later subtasks). Production rendering (`Off`, `MeshletIds`,
        // `InstanceIds`, `CullPassthrough`, `OnlyLod0`, `OnlyRoots`)
        // leaves the uniform at 0 so the hot path skips the atomic
        // increment. The density texture is allocated in lockstep
        // with the stage's render targets; it is `Some` whenever
        // the vbuf64 path is selectable.
        let density_mode: u32 = match debug_mode {
            MeshletDebugMode::TriangleDensity
            | MeshletDebugMode::HiZRejected
            | MeshletDebugMode::BackfaceRejected => 1,
            MeshletDebugMode::Overdraw => 2,
            _ => 0,
        };
        // 🔴 Shadow pages (#866), BEFORE the fused pass and not after
        // it. `vbuf64.render` rasterises and lights in one fragment
        // shader, so whatever it samples has to be finished by the time
        // it starts.
        //
        // The marking reads the depth buffer, which at this point still
        // holds the LAST frame — the fused pass is what clears and
        // refills it. That is Epic's arrangement and the trade is
        // deliberate: last frame's depth decides which pages EXIST,
        // which is off by however far the camera moved in a frame and
        // costs a page at the edge of the screen; this frame's geometry
        // fills them, which is what the shading compares against.
        //
        // The other way round — marking after the shading — meant the
        // atlas was a frame old, so a moving object was compared against
        // its OWN caster from the previous frame and shadowed itself.
        self.record_page_marking(
            device,
            queue,
            &mut encoder,
            resources,
            view_id,
            unjittered_view_proj,
            cam_pos,
            scene_params,
            meshlet_bg,
            debug_mode,
        );
        self.bind_page_shadows(device, resources, view_id);
        let density_view = self.views[view_id]
            .triangle_density_view
            .as_ref()
            .expect("density texture must be allocated whenever vbuf64 path is active");
        // Stage 1 (Raster — R64 atomic vbuf is raster + fused shading
        // in one fragment shader; no separate deferred pass on this
        // path).
        if timer_slot.is_some() {
            self.gpu_timers.write_stage_start(&mut encoder, 1);
        }
        let material_pipeline = resources.get::<crate::material::MaterialPipeline>();
        // Hoisted out of the call below: the page-marking debug view
        // needs it too, to divide out what the tonemap multiplies back
        // in (#866).
        let exposure = resources
            .get::<kooch_lighting::Exposure>()
            .copied()
            .unwrap_or_default()
            .multiplier();
        // 🔴 Braced, like `upload instances`: a `profiling::scope!`
        // lives to the end of its block, and mid-function this one
        // reported the overlay dispatch, the readbacks and `Queue::
        // submit` as part of the raster. The CPU cost of submitting is
        // not the cost of shading.
        // 🔴 The block's value, not a mutable binding written from
        // inside it: DLSS hands back a command buffer that has to reach
        // the single submit at the bottom of this function (#536).
        let vbuf64 = self.views[view_id]
            .vbuf64_stage
            .as_ref()
            .expect("path selected only when vbuf64_stage is Some");
        let frame_dlss_commands = {
            profiling::scope!("raster + shade (fused)");
            // The prime suspect for the 96 % (#769): one fragment shader
            // doing both the raster and the whole lighting evaluation, over
            // every pixel the scene covers.
            let shade_query = scopes.map(|s| s.begin("raster + shade", &mut encoder));
            let dlss_commands = vbuf64.render(
                device,
                queue,
                &mut encoder,
                &self.views[view_id].depth_view,
                &self.views[view_id].depth_sample_view,
                &self.views[view_id].color_view,
                density_view,
                density_mode,
                meshlet_bg,
                material_pipeline.as_deref(),
                self.lights.bind_group(),
                &self.views[view_id].cull,
                &self.scene,
                view_proj,
                unjittered_view_proj,
                contact,
                debug_mode.as_u32(),
                // #732 — the tonemap is its own pass now, so the scalar
                // it used to read out of the Inti uniform is passed to
                // the stage instead.
                exposure,
                /* clear_depth */ true,
                scopes.as_deref(),
                shade_query.as_ref(),
                dlss_runtime.as_deref(),
            );
            if let (Some(scopes), Some(query)) = (scopes, shade_query) {
                scopes.end(&mut encoder, query);
            }
            // 🔴 Kept until the submit below, where it goes in the SAME
            // submission and immediately after this encoder. DLSS
            // records into wgpu's Vulkan command pool behind its back;
            // submitting it apart from, or before, the frame that fed
            // it is undefined behaviour by the crate's own contract.
            dlss_commands
        };

        // 🔴 The frame is CUT here when DLSS ran, and the rest of it —
        // the debug overlay below, the GPU timer resolve, the final
        // submit — continues in the encoder the stage handed back.
        // Everything downstream reads the upscaled image, and DLSS has
        // not written it until its own buffer reaches the queue.
        if let Some(deferred) = frame_dlss_commands {
            queue.submit([encoder.finish(), deferred.dlss]);
            encoder = deferred.post;
        }
        // The debug paint, which is all that is left here. See
        // `PageMarker::record_paint`: the marking itself moved to the
        // top of the frame, but the paint writes the view's FINAL colour
        // and has to land after the fused pass and after the DLSS cut.
        self.record_page_paint(&mut encoder, view_id);

        if timer_slot.is_some() {
            self.gpu_timers.write_stage_end(&mut encoder, 1);
            self.gpu_timers.write_stage_start(&mut encoder, 2);
        }

        // Reject-reason overlay (#454.4). Dispatched only when the
        // current debug mode is one the cull pass tagged into
        // `reject_reasons[]` AND the stage owns the overlay pipeline
        // (i.e. `MeshletDebugCaps::supports_texture_atomic` was true
        // at construction). The dropdown filter prevents the user
        // from selecting a mode for which one of those is false, but
        // we re-check defensively so a runtime resource swap can't
        // dispatch into nothing.
        if let (Some(reject_code), Some(overlay), Some(gpu_pool)) = (
            debug_mode.reject_reason_code(),
            self.reject_overlay.as_ref(),
            self.gpu_pool.as_ref(),
        ) {
            let total_threads = scene_params.instance_count * scene_params.meshlets_per_mesh;
            let reason = match reject_code {
                2 => crate::meshlet::RejectReason::Frustum,
                3 => crate::meshlet::RejectReason::Backface,
                4 => crate::meshlet::RejectReason::HiZ,
                _ => crate::meshlet::RejectReason::Skipped,
            };
            overlay.dispatch(
                device,
                queue,
                &mut encoder,
                &self.views[view_id].color_view,
                &self.cull_pipelines,
                &self.views[view_id].cull,
                &self.scene,
                gpu_pool,
                view_proj,
                self.views[view_id].render_size,
                reason,
                /* line_thickness_px */ 2,
                total_threads,
            );
        }

        // #454.6 — stage-counter readback is gated to debug-active
        // frames. The cull shader only writes the SSBO when
        // `params.debug_active != 0`; if it didn't this frame, the
        // copy would just shuttle stale zeros (or last-frame's
        // values when the user just toggled the mode off). Skip
        // entirely in that case.
        let stage_slot = if cull_params.debug_active != 0 {
            let slot = self.stage_counters.acquire_slot();
            if let Some(idx) = slot {
                self.stage_counters.write_copy(
                    &mut encoder,
                    self.views[view_id].cull.stage_counters_buffer(),
                    idx,
                );
            }
            slot
        } else {
            None
        };

        if let Some(slot_idx) = timer_slot {
            // Stage 2 (Overlay + stage-counters copy). Closes the
            // last stage and emits the single resolve_and_copy for
            // every timestamp in the slot.
            self.gpu_timers.write_stage_end(&mut encoder, 2);
            self.gpu_timers.resolve_and_copy(&mut encoder, slot_idx);
        }
        queue.submit(std::iter::once(encoder.finish()));
        // The counters' ring maps after the submit, the way every other
        // readback here does.
        self.report_page_marking(resources);
        self.report_page_raster(resources);
        if let Some(slot_idx) = timer_slot {
            self.gpu_timers.submit_readback(slot_idx);
        }
        if let Some(slot_idx) = stage_slot {
            self.stage_counters.submit_readback(slot_idx);
        }

        let pool = self.pipeline.pool();
        let pool_meshlets_total = pool.meshlets.len() as u32;
        let pool_meshlets_roots = pool
            .meshlets
            .iter()
            .filter(|m| m.parent_meshlet_index == crate::meshlet::asset::MESHLET_ROOT_PARENT)
            .count() as u32;
        // R64 path: cull + clear + raster + deferred.
        let meshlet_draw_calls = if instance_count == 0 { 0 } else { 4 };
        let stage_timings = self.gpu_timers.last_frame_stage_timings().and_then(|t| {
            if t.len() == 3 {
                Some([("Cull", t[0]), ("Raster", t[1]), ("Overlay", t[2])])
            } else {
                None
            }
        });
        MeshletRenderStats {
            instances_uploaded: instance_count,
            cull_threads: scene_params.instance_count * scene_params.meshlets_per_mesh,
            cam_pos: cam_pos.to_array(),
            pool_meshlets_total,
            pool_meshlets_roots,
            gpu_frame_ms: self.gpu_timers.last_frame_ms(),
            draw_calls: meshlet_draw_calls,
            // Only when this frame asked for them. The cache holds
            // whatever the last debug-active frame read, and handing
            // that to the HUD draws a number from an unknown moment as
            // if it described the frame on screen (#703).
            cluster_occupancy: self.lights.clusters().occupancy(),
            // 🔴 THIS view's counts, not the last readback to land. The
            // stats a view publishes have to describe the camera that
            // produced them, or a panel drawn beside one viewport
            // reports the other one's frustum.
            page_marking: self.page_marking_for(view_id),
            page_raster: self.page_raster_for(view_id),
            cull_stage_counts: if cull_params.debug_active != 0 {
                self.stage_counters.last_frame_counts()
            } else {
                None
            },
            stage_timings,
        }
    }
}

//! Legacy R32 + Hi-Z 2-pass orchestrator (#445 + #486 SPD + #488 AABB
//! cull). Used by [`MeshletRenderStage::render`] when the device does
//! not expose the atomic R64 vbuf feature bundle.
//!
//! 6 logical passes per frame: cull A, raster A, SPD pyramid build,
//! cull B, raster B, deferred shade. Owns the encoder it receives,
//! creates additional encoders for the SPD build + pass-B raster,
//! issues all submits, and returns the per-frame [`MeshletRenderStats`].

use glam::{Mat4, Vec3};

use kooch_core::resource::Resources;

use crate::meshlet::cull::CullParams;
use crate::meshlet::debug::MeshletDebugMode;
use crate::meshlet::scene::SceneCullParams;

use super::super::{MeshletRenderStage, MeshletRenderStats, ViewId};

impl MeshletRenderStage {
    /// Legacy R32 + Hi-Z 2-pass orchestrator. Stats report `draw_calls
    /// = 6`. See [`Self::render`] for the dispatch decision and the
    /// prelude that builds `cull_params` / `scene_params` /
    /// `meshlet_bg` / `material_bg` / `timer_slot`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_path_hi_z_two_pass(
        &mut self,
        view_id: ViewId,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        resources: &Resources,
        view_proj: Mat4,
        cam_pos: Vec3,
        cull_params: &CullParams,
        scene_params: &SceneCullParams,
        meshlet_bg: &wgpu::BindGroup,
        material_bg: &wgpu::BindGroup,
        contact: &crate::contact_shadow::ContactShadowUbo,
        timer_slot: Option<usize>,
        instance_count: u32,
    ) -> MeshletRenderStats {
        // Triple-buffer arena rotation (#445 PR #479 Mesa radv
        // workaround): pick the slot 2 frames stale, clear it, and
        // park this frame's bind groups there so they outlive GPU
        // execution.
        self.frame_bind_groups_index = (self.frame_bind_groups_index + 1) % 3;
        let arena_idx = self.frame_bind_groups_index;
        self.frame_bind_groups[arena_idx].clear();
        self.views[view_id].retired_pyramids[arena_idx].clear();

        if self.views[view_id].hiz_prev.is_none() {
            let pyr = crate::hi_z::HiZ::new(
                device,
                self.views[view_id].size.0,
                self.views[view_id].size.1,
            );
            if let Some(tracker) = &self.vram_tracker {
                tracker.add(pyr.byte_size());
            }
            self.views[view_id].hiz_prev = Some(pyr);
        }
        if self.views[view_id].hiz_curr.is_none() {
            let pyr = crate::hi_z::HiZ::new(
                device,
                self.views[view_id].size.0,
                self.views[view_id].size.1,
            );
            if let Some(tracker) = &self.vram_tracker {
                tracker.add(pyr.byte_size());
            }
            self.views[view_id].hiz_curr = Some(pyr);
        }

        // First-frame init under reversed-Z: hiz_prev needs to be
        // seeded so pass A's first sample doesn't read undefined
        // R32Float bytes. Clear depth to 0.0 (= far in reversed-Z)
        // then SPD-build over it; the resulting pyramid says "every
        // tile's farthest fragment is at the far plane", which is
        // the conservative "nothing in front" baseline.
        if !self.views[view_id].hi_z_initialized {
            {
                let mut clear_enc =
                    device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("meshlet_hi_z_init_depth_clear_encoder"),
                    });
                let _depth_clear = clear_enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("meshlet_hi_z_first_frame_depth_clear"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.views[view_id].depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                drop(_depth_clear);
                queue.submit(std::iter::once(clear_enc.finish()));
            }
            let mut init_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("meshlet_hi_z_init_build_encoder"),
            });
            {
                let hiz_prev = self.views[view_id]
                    .hiz_prev
                    .as_ref()
                    .expect("just allocated");
                hiz_prev.init_to_far(
                    device,
                    &mut init_enc,
                    &self.views[view_id].depth_sample_view,
                    &mut self.frame_bind_groups[arena_idx],
                );
            }
            queue.submit(std::iter::once(init_enc.finish()));
            self.views[view_id].hi_z_initialized = true;
        }

        let (hiz_w, hiz_h, mip_count) = {
            let hiz_prev = self.views[view_id]
                .hiz_prev
                .as_ref()
                .expect("allocated above");
            let (w, h) = hiz_prev.dimensions();
            (w, h, hiz_prev.mip_count())
        };
        let hi_z_params =
            crate::meshlet::dispatcher::HiZTestParams::new(view_proj, hiz_w, hiz_h, mip_count);

        // Pass A: AABB-based cull against hiz_prev.
        {
            let gpu_pool = self.gpu_pool.as_ref().expect("checked by render() prelude");
            let hiz_prev = self.views[view_id]
                .hiz_prev
                .as_ref()
                .expect("allocated above");
            let hiz_prev_view = hiz_prev.full_view();
            self.views[view_id].cull.dispatch_scene_pool_atomic_hi_z(
                &self.cull_pipelines,
                device,
                queue,
                &mut encoder,
                gpu_pool,
                &self.scene,
                cull_params,
                scene_params,
                &hi_z_params,
                hiz_prev_view,
                &mut self.frame_bind_groups[arena_idx],
            );
        }
        // Raster A: clear vbuf + depth, draw pass A's survivors.
        self.rasterizer.render_scene(
            device,
            queue,
            &mut encoder,
            &self.views[view_id].vbuf_view,
            &self.views[view_id].depth_view,
            meshlet_bg,
            &self.views[view_id].cull,
            &self.scene,
            view_proj,
            0,
            /* clear */ true,
        );
        // Stage 0 (Pass A) closes here. `render()`'s prelude already
        // emitted `write_start` which lands on stage 0.
        if timer_slot.is_some() {
            self.gpu_timers.write_stage_end(&mut encoder, 0);
        }
        queue.submit(std::iter::once(encoder.finish()));

        let mut build_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("meshlet_hi_z_build_encoder"),
        });
        // Stage 1 (Hi-Z SPD build).
        if timer_slot.is_some() {
            self.gpu_timers.write_stage_start(&mut build_enc, 1);
        }
        {
            let hiz_curr = self.views[view_id]
                .hiz_curr
                .as_ref()
                .expect("allocated above");
            hiz_curr.build_from_depth(
                device,
                &mut build_enc,
                &self.views[view_id].depth_sample_view,
                &mut self.frame_bind_groups[arena_idx],
            );
        }
        if timer_slot.is_some() {
            self.gpu_timers.write_stage_end(&mut build_enc, 1);
        }
        queue.submit(std::iter::once(build_enc.finish()));

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("meshlet_render_stage_encoder_pass_b"),
        });
        // Stage 2 (Pass B = cull B + raster B + deferred shade).
        if timer_slot.is_some() {
            self.gpu_timers.write_stage_start(&mut encoder, 2);
        }
        {
            let gpu_pool = self.gpu_pool.as_ref().expect("checked by render() prelude");
            let hiz_curr = self.views[view_id]
                .hiz_curr
                .as_ref()
                .expect("allocated above");
            let hiz_curr_view = hiz_curr.full_view();
            self.views[view_id].cull.dispatch_cull_pass_b(
                &self.cull_pipelines,
                device,
                queue,
                &mut encoder,
                gpu_pool,
                &self.scene,
                &hi_z_params,
                hiz_curr_view,
                &mut self.frame_bind_groups[arena_idx],
            );
        }
        // Raster B: load (preserve pass-A vbuf + depth) and draw
        // pass A + B contributions.
        self.rasterizer.render_scene(
            device,
            queue,
            &mut encoder,
            &self.views[view_id].vbuf_view,
            &self.views[view_id].depth_view,
            meshlet_bg,
            &self.views[view_id].cull,
            &self.scene,
            view_proj,
            0,
            /* clear */ false,
        );
        let debug_mode = resources
            .get::<MeshletDebugMode>()
            .copied()
            .unwrap_or_default()
            .as_u32();
        self.deferred.shade_scene(
            device,
            queue,
            &mut encoder,
            &self.views[view_id].vbuf_view,
            &self.views[view_id].depth_sample_view,
            &self.views[view_id].color_view,
            meshlet_bg,
            material_bg,
            &self.views[view_id].cull,
            &self.scene,
            self.lights.bind_group(),
            view_proj,
            contact,
            self.views[view_id].size,
            debug_mode,
        );
        if let Some(slot_idx) = timer_slot {
            self.gpu_timers.write_stage_end(&mut encoder, 2);
            self.gpu_timers.resolve_and_copy(&mut encoder, slot_idx);
        }
        queue.submit(std::iter::once(encoder.finish()));
        if let Some(slot_idx) = timer_slot {
            self.gpu_timers.submit_readback(slot_idx);
        }

        // Rotate pyramids: next frame's hiz_prev = this frame's hiz_curr.
        self.swap_hi_z_pyramids();

        let pool = self.pipeline.pool();
        let pool_meshlets_total = pool.meshlets.len() as u32;
        let pool_meshlets_roots = pool
            .meshlets
            .iter()
            .filter(|m| m.parent_meshlet_index == crate::meshlet::asset::MESHLET_ROOT_PARENT)
            .count() as u32;

        // Hi-Z 2-pass orchestrator (#445 + #486 + #488): 6 logical
        // passes per frame — cull A, raster A, SPD pyramid build,
        // cull B, raster B, deferred shade.
        let meshlet_draw_calls = if instance_count == 0 { 0 } else { 6 };

        let stage_timings = self.gpu_timers.last_frame_stage_timings().and_then(|t| {
            if t.len() == 3 {
                Some([("Pass A", t[0]), ("Hi-Z", t[1]), ("Pass B", t[2])])
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
            // The Hi-Z 2-pass cull entry doesn't write
            // `stage_counters[]` in #454.6 scope — wiring it is
            // tracked alongside the SPD-backed orchestrator
            // follow-up (#445). Surface the cached value so the
            // editor stats overlay still shows the LATEST counts
            // from any frame the R64 path also ran (which it
            // doesn't on this branch — stays None on legacy R32).
            // Only when this frame asked for them. The cache holds
            // whatever the last debug-active frame read, and handing
            // that to the HUD draws a number from an unknown moment as
            // if it described the frame on screen (#703).
            cull_stage_counts: if cull_params.debug_active != 0 {
                self.stage_counters.last_frame_counts()
            } else {
                None
            },
            stage_timings,
        }
    }
}

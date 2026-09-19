//! The SHADING half of a frame: motion, material passes, transparency, upscale and tonemap.

use super::*;

impl Vbuf64Stage {
    /// The SHADING half. See [`Self::render_geometry`] for why the
    /// two are apart.
    #[allow(clippy::too_many_arguments)]
    pub fn render_shading(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth_view: &wgpu::TextureView,
        depth_sample_view: &wgpu::TextureView,
        color_view: &wgpu::TextureView,
        density_view: &wgpu::TextureView,
        meshlet_bg: &wgpu::BindGroup,
        material_pipeline: Option<&crate::material::MaterialPipeline>,
        lights_bg: &wgpu::BindGroup,
        cull: &MeshletCull,
        scene: &MeshletScene,
        // #481 — the jittered matrix the raster and every reconstruction
        // off its visibility buffer use.
        view_proj: glam::Mat4,
        // …and the camera's own, which only the motion vectors read. Equal to `view_proj` whenever
        // TAA is off. See [`Self::next_jitter`] for why they are handed down as a pair rather than
        // one being derived here.
        unjittered_view_proj: glam::Mat4,
        contact: &crate::contact_shadow::ContactShadowUbo,
        debug_mode: u32,
        // #1159 — seconds since the engine started, for a surface that moves. It rides in the
        // padding the screen UBO already carried, so no binding changed to carry it.
        time: f32,
        // #732 — the tonemap moved out of the shading shader, so the
        // scalar it used to read from the Inti uniform has to reach
        // the pass that applies it now.
        exposure: f32,
        // the caller's `raster + shade`.
        scopes: Option<&kooch_core::gpu::GpuScopes>,
        parent: Option<&kooch_core::gpu::GpuQuery>,
        // systems have already removed `GpuContext` from `Resources` to get at. `None` in every
        // build and on every adapter that has no DLSS, which is most of them.
        dlss_runtime: Option<&kooch_core::gpu::DlssRuntime>,
        // #452 — the transparent instances' meshlets, far to near.
        forward_list: &ForwardList,
    ) -> Option<Deferred> {
        // 🔴 DLSS hands back a command buffer of its own that has to be
        // submitted immediately after this frame's encoder, so it
        // travels all the way out of here rather than being recorded.
        let mut dlss_commands: Option<wgpu::CommandBuffer> = None;
        // Motion vectors, right after the raster that fills the vbuf they read and before anything
        // that shades (#481).
        if self.needs_motion() {
            let query = match (scopes, parent) {
                (Some(s), Some(p)) => Some(s.begin_child("motion vectors", encoder, p)),
                (Some(s), None) => Some(s.begin("motion vectors", encoder)),
                _ => None,
            };
            self.motion.dispatch(
                device,
                queue,
                encoder,
                &self.vbuf_view,
                meshlet_bg,
                cull.visible_meshlets_buffer(),
                scene.instance_buffer(),
                scene.previous_transform_buffer(),
                view_proj,
                unjittered_view_proj,
                self.size,
            );
            if let (Some(scopes), Some(query)) = (scopes, query) {
                scopes.end(encoder, query);
            }
        }
        // Colorize debug modes (ids / heatmaps / cull passthrough) render through the fullscreen
        // debug fragment pass.
        if debug_resolve::is_colorize_mode(debug_mode) {
            self.debug_resolve.draw(
                device,
                queue,
                encoder,
                &self.vbuf_view,
                color_view,
                density_view,
                cull,
                self.size,
                debug_mode,
            );
        } else if let Some(pipeline) = material_pipeline {
            // The label names the path, so the capture answers "which
            // one ran" without anybody having to trust a log line.
            let label = if self.compute_enabled {
                self.shading_rate.scope_label()
            } else {
                "shade: fragment"
            };
            let query = match (scopes, parent) {
                (Some(s), Some(p)) => Some(s.begin_child(label, encoder, p)),
                (Some(s), None) => Some(s.begin(label, encoder)),
                _ => None,
            };
            if self.compute_enabled {
                // At a reduced rate the shading writes the upsample's
                // own targets; at full rate it writes the screen and the
                // id target is bound but never stored to.
                let half = self.shading_rate.needs_upsample();
                if half {
                    self.upsample.clear_ids(encoder);
                }
                // 🔴 Neither branch writes `color_view` any more: the
                // compute path shades into HDR and the tonemap pass
                // below puts it on screen (#732).
                let shade_target = if half {
                    self.upsample.color_view()
                } else {
                    self.tonemap.hdr_view()
                };
                self.compute_shade.shade(
                    device,
                    queue,
                    encoder,
                    &self.vbuf_view,
                    depth_sample_view,
                    shade_target,
                    self.upsample.id_view(),
                    meshlet_bg,
                    cull,
                    scene,
                    pipeline,
                    lights_bg,
                    view_proj,
                    contact,
                    self.size,
                    self.shading_rate,
                    self.mip_bias_scale(),
                    time,
                    debug_mode,
                    scopes,
                    query.as_ref(),
                );
            } else if self.size != self.output_size {
                // 🔴 The fragment path shades a render-sized material depth buffer onto the
                // window-sized image, and wgpu refuses a pass whose attachments disagree — it
                // discards the whole thing, so the frame is not soft, it is absent.
                warn_once_about_transitional_frame(self.size, self.output_size);
            } else {
                self.two_pass.shade(
                    device,
                    queue,
                    encoder,
                    &self.vbuf_view,
                    &self.material_depth_view,
                    depth_sample_view,
                    color_view,
                    meshlet_bg,
                    cull,
                    scene,
                    pipeline,
                    lights_bg,
                    view_proj,
                    contact,
                    self.size,
                    time,
                    debug_mode,
                    scopes,
                    query.as_ref(),
                );
            }
            if let (Some(scopes), Some(query)) = (scopes, query) {
                scopes.end(encoder, query);
            }
            // Its own scope, and a sibling of the shading rather than a child of it: the whole
            // question this issue asks is whether what the reduced rate saves survives what putting
            // it back on screen costs. Two numbers a capture can subtract.
            if self.compute_enabled && self.shading_rate.needs_upsample() {
                let query = match (scopes, parent) {
                    (Some(s), Some(p)) => Some(s.begin_child("shade: upsample", encoder, p)),
                    (Some(s), None) => Some(s.begin("shade: upsample", encoder)),
                    _ => None,
                };
                self.upsample.draw(
                    device,
                    queue,
                    encoder,
                    &self.vbuf_view,
                    self.tonemap.hdr_view(),
                    self.size,
                );
                if let (Some(scopes), Some(query)) = (scopes, query) {
                    scopes.end(encoder, query);
                }
            }
            if self.compute_enabled {
                let query = match (scopes, parent) {
                    (Some(s), Some(p)) => Some(s.begin_child("transparent", encoder, p)),
                    (Some(s), None) => Some(s.begin("transparent", encoder)),
                    _ => None,
                };
                let frame = forward::ForwardFrame {
                    device,
                    queue,
                    encoder: &mut *encoder,
                    target: self.tonemap.hdr_view(),
                    depth: depth_view,
                    depth_sample: depth_sample_view,
                    vbuf: &self.vbuf_view,
                    meshlet_bg,
                    scene,
                    materials: pipeline,
                    lights_bg,
                    view_proj,
                    contact,
                    size: self.size,
                    mip_bias_scale: self.mip_bias_scale(),
                    time,
                };
                self.forward.lock().unwrap_or_else(|e| e.into_inner()).draw(
                    frame,
                    self.two_pass.layouts(),
                    forward_list,
                );
                if let (Some(scopes), Some(query)) = (scopes, query) {
                    scopes.end(encoder, query);
                }
            } else if !forward_list.is_empty() {
                static WARNED: std::sync::Once = std::sync::Once::new();
                WARNED.call_once(|| {
                    tracing::warn!(
                        "transparent surfaces are drawn on the compute shading path only"
                    );
                });
            }
            if self.compute_enabled {
                // The temporal resolve, between the radiance and the curve (#481). Skipped on the
                // debug views: they hand back display-referred false colour, and averaging a
                // cluster index with last frame's produces a number that indexes nothing.
                let mut source = self.tonemap.hdr_view();
                // While `source` is the render-resolution HDR texture, the tonemap must stretch its
                // written region to the target — a debug view that skips the upscaler used to show
                // the scene at half size in a corner.
                let mut source_scale = [
                    self.size.0 as f32 / self.output_size.0.max(1) as f32,
                    self.size.1 as f32 / self.output_size.1.max(1) as f32,
                ];
                if self.technique.is_temporal() && !replaces_shading(debug_mode) {
                    // 🔴 The scope carries the technique's name rather than a shared "temporal". A
                    // capture has to say WHICH one cost what, or the A/B that decides between them
                    // is two numbers under one label.
                    let label = match self.technique {
                        crate::quality::UpscaleTechnique::Sgsr2 => "sgsr2",
                        crate::quality::UpscaleTechnique::Fsr3 => "fsr3",
                        crate::quality::UpscaleTechnique::Dlss => "dlss",
                        _ => "taa",
                    };
                    let query = match (scopes, parent) {
                        (Some(s), Some(p)) => Some(s.begin_child(label, encoder, p)),
                        (Some(s), None) => Some(s.begin(label, encoder)),
                        _ => None,
                    };
                    // Strategy, dispatched by value: one match per
                    // frame, no vtable, and the compiler checks that a
                    // new technique is handled here.
                    source = match self.technique {
                        crate::quality::UpscaleTechnique::Sgsr2 => self.sgsr2.draw(
                            device,
                            queue,
                            encoder,
                            self.upscale_inputs(depth_sample_view, exposure, 0, None, None),
                        ),
                        crate::quality::UpscaleTechnique::Fsr3 => self.fsr3.draw(
                            device,
                            queue,
                            encoder,
                            self.upscale_inputs(
                                depth_sample_view,
                                exposure,
                                fsr3_debug_stage(debug_mode),
                                scopes,
                                query.as_ref(),
                            ),
                        ),
                        // 🔴 The one technique that can decline. A build without the feature, an AMD
                        // card, or an SDK call that failed all land in the same place: the engine's
                        // own resolve, so the frame is still antialiased and still arrives.
                        crate::quality::UpscaleTechnique::Dlss => {
                            let inputs =
                                self.upscale_inputs(depth_sample_view, exposure, 0, None, None);
                            match self
                                .dlss
                                .draw(device, queue, encoder, dlss_runtime, &inputs)
                            {
                                Some((view, commands)) => {
                                    dlss_commands = Some(commands);
                                    view
                                }
                                None => self.taa.draw(
                                    device,
                                    queue,
                                    encoder,
                                    self.tonemap.hdr_view(),
                                    self.motion.view(),
                                    depth_sample_view,
                                    exposure,
                                ),
                            }
                        }
                        _ => self.taa.draw(
                            device,
                            queue,
                            encoder,
                            self.tonemap.hdr_view(),
                            self.motion.view(),
                            depth_sample_view,
                            exposure,
                        ),
                    };
                    if let (Some(scopes), Some(query)) = (scopes, query) {
                        scopes.end(encoder, query);
                    }
                    source_scale = [1.0, 1.0];
                }
                // 🔴 Everything below reads the image the resolve just produced — and when that
                // resolve was DLSS, it has not happened yet.
                let mut post = dlss_commands.as_ref().map(|_| {
                    device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("kooch_post_upscale"),
                    })
                });
                let encoder: &mut wgpu::CommandEncoder = post.as_mut().unwrap_or(&mut *encoder);

                // 🔴 Sharpening reads a FINISHED image (#481 step 5), so when it runs the tonemap
                // resolves into its texture instead of into the window and it is RCAS that writes
                // what is presented.
                let sharpening = self.sharpening;
                let sharpening_runs = sharpening > 0 && !is_debug_view(debug_mode);
                let tonemap_target = if sharpening_runs {
                    self.sharpen.input_view()
                } else {
                    color_view
                };
                // HDR to the image. Its own scope: it is a full-screen pass that did not exist
                // before, and "the tonemap moved" has to be answerable with a number rather than an
                // argument.
                let query = match (scopes, parent) {
                    (Some(s), Some(p)) => Some(s.begin_child("tonemap", encoder, p)),
                    (Some(s), None) => Some(s.begin("tonemap", encoder)),
                    _ => None,
                };
                self.tonemap.draw(
                    queue,
                    device,
                    encoder,
                    source,
                    tonemap_target,
                    exposure,
                    // The Inti debug views hand back display-ready colour.
                    !is_display_referred(debug_mode),
                    source_scale,
                );
                if let (Some(scopes), Some(query)) = (scopes, query) {
                    scopes.end(encoder, query);
                }
                // Its own scope, because the whole argument for this pass is that ~0.2 ms buys back
                // what reconstruction takes away, and an argument of that shape is settled by two
                // numbers rather than by an opinion.
                if sharpening_runs {
                    let query = match (scopes, parent) {
                        (Some(s), Some(p)) => Some(s.begin_child("rcas", encoder, p)),
                        (Some(s), None) => Some(s.begin("rcas", encoder)),
                        _ => None,
                    };
                    self.sharpen
                        .draw(device, queue, encoder, color_view, sharpening);
                    if let (Some(scopes), Some(query)) = (scopes, query) {
                        scopes.end(encoder, query);
                    }
                }
                return dlss_commands
                    .zip(post)
                    .map(|(dlss, post)| Deferred { dlss, post });
            }
        }
        None
    }
}

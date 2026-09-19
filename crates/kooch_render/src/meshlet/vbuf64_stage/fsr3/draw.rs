//! One frame of the schedule, dispatch by dispatch.

use super::*;

impl Fsr3 {
    /// Runs the whole schedule and returns the resolved image.
    ///
    /// One compute pass per dispatch rather than one pass with six
    /// dispatches in it: every stage reads what the one before it
    /// wrote, and a pass boundary is the barrier that guarantees it.
    ///
    /// ⚠️ All of it lands under the single `fsr3` GPU scope its caller
    /// opened. Splitting that into six needs the profiler threaded
    /// through `UpscaleInputs`, which is a change to the seam both
    /// techniques share — worth doing once there is a number that says
    /// which dispatch to look at.
    pub(in crate::meshlet::vbuf64_stage) fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        inputs: UpscaleInputs<'_>,
    ) -> &wgpu::TextureView {
        let mut state = self.state.lock().expect("fsr3 history lock");
        let previous = state.index;
        let target = 1 - previous;

        let render = (
            self.render_size.0.max(1) as f32,
            self.render_size.1.max(1) as f32,
        );
        let out = (
            self.output_size.0.max(1) as f32,
            self.output_size.1.max(1) as f32,
        );
        let exposure = if inputs.exposure > 0.0 {
            inputs.exposure
        } else {
            1.0
        };

        // 🔴 One line per change of debug step, and it exists because a
        // black frame in the editor could equally mean this pass never
        // ran, ran on an empty input, or ran on an exposure that puts
        // the whole scene under a bit. The test rig cannot tell those
        // apart from the outside and neither could I.
        if state.logged_stage != Some(inputs.debug_stage) {
            state.logged_stage = Some(inputs.debug_stage);
            tracing::info!(
                stage = inputs.debug_stage,
                render = ?self.render_size,
                output = ?self.output_size,
                exposure,
                near = inputs.near,
                jitter = ?inputs.jitter,
                frame = state.frame_index,
                "fsr3 draw",
            );
        }

        queue.write_buffer(
            &self.ubo,
            0,
            bytemuck::bytes_of(&Fsr3Ubo {
                render_size: [render.0, render.1],
                output_size: [out.0, out.1],
                render_size_rcp: [1.0 / render.0, 1.0 / render.1],
                output_size_rcp: [1.0 / out.0, 1.0 / out.1],
                jitter: [inputs.jitter.x, inputs.jitter.y],
                prev_jitter: [state.prev_jitter.x, state.prev_jitter.y],
                downscale: [render.0 / out.0, render.1 / out.1],
                near: inputs.near,
                exposure,
                reset: f32::from(u8::from(state.reset)),
                frame_index: state.frame_index,
                // The history was resolved at the previous exposure, so
                // it has to be rescaled before it is blended with a
                // frame at this one.
                delta_pre_exposure: state.prev_exposure / exposure,
                jitter_sequence_length: inputs.jitter_phases.max(1.0),
                debug: inputs.debug_stage,
                _pad: [0.0; 3],
            }),
        );

        let t = &self.targets;
        let texture = wgpu::BindingResource::TextureView;

        let inputs_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fsr3_prepare_inputs_bg"),
            layout: &self.prepare_inputs_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.ubo.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: texture(inputs.depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: texture(inputs.motion),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: texture(inputs.color),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: texture(&t.reconstructed_depth.view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: texture(&t.dilated.view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: texture(&t.dilated_depth.view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: texture(&t.current_luma.view),
                },
            ],
        });
        let reduce_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fsr3_reduce_bg"),
            layout: &self.reduce_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.ubo.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: texture(&t.dilated.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: texture(&t.farthest_mip1.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: texture(&t.new_locks.view),
                },
            ],
        });
        let reactivity_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fsr3_reactivity_bg"),
            layout: &self.reactivity_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.ubo.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: texture(&t.dilated.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: texture(&t.dilated_depth.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: texture(&t.reconstructed_depth.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: texture(&t.current_luma.view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: texture(t.accumulation.view(previous)),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&self.linear),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: texture(&t.reactive_masks.view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: texture(t.accumulation.view(target)),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: texture(&t.new_locks.view),
                },
            ],
        });
        let instability_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fsr3_instability_bg"),
            layout: &self.instability_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.ubo.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: texture(&t.dilated.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: texture(&t.current_luma.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: texture(t.luma_history.view(previous)),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: texture(&t.reactive_masks.view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.linear),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: texture(t.luma_history.view(target)),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: texture(&t.luma_instability.view),
                },
            ],
        });
        let accumulate_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fsr3_accumulate_bg"),
            layout: &self.accumulate_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.ubo.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: texture(inputs.color),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: texture(&t.dilated.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: texture(&t.reactive_masks.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: texture(&t.luma_instability.view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: texture(&t.farthest_mip1.view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: texture(t.history.view(previous)),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(&self.linear),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: texture(&t.new_locks.view),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: texture(t.history.view(target)),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: texture(&t.output.view),
                },
            ],
        });

        let render_groups = groups(self.render_size);
        let half_groups = groups((self.render_size.0 / 2, self.render_size.1 / 2));
        let output_groups = groups(self.output_size);

        // 🎯 One GPU scope per dispatch, nested inside the caller's.
        // The alternative measured 15.164 ms for the six of them
        // together on the handheld — a number that condemns the
        // technique and does not say which pass to open.
        let (scopes, parent) = (inputs.scopes, inputs.parent);
        let mut dispatch = |label: &'static str,
                            pipeline: &wgpu::ComputePipeline,
                            bind_group: &wgpu::BindGroup,
                            count: (u32, u32)| {
            let query = match (scopes, parent) {
                (Some(s), Some(p)) => Some(s.begin_child(label, encoder, p)),
                (Some(s), None) => Some(s.begin(label, encoder)),
                _ => None,
            };
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(label),
                    timestamp_writes: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, bind_group, &[]);
                pass.dispatch_workgroups(count.0, count.1, 1);
            }
            if let (Some(scopes), Some(query)) = (scopes, query) {
                scopes.end(encoder, query);
            }
        };

        // The scatter only writes where something reprojects to, so
        // anything it misses would keep the previous frame's depth and
        // report a false occlusion.
        dispatch(
            "clear depth",
            &self.clear_reconstructed,
            &inputs_bg,
            render_groups,
        );
        // 🎯 The locks need no such clear: `accumulate` zeroes every
        // output pixel it consumes, and wgpu hands over a texture that
        // is already zero. Only a reset — where `accumulate` has not run
        // against these targets at all — has to do it explicitly.
        if state.reset {
            dispatch(
                "clear locks",
                &self.clear_new_locks,
                &reduce_bg,
                output_groups,
            );
        }
        dispatch(
            "prepare inputs",
            &self.prepare_inputs,
            &inputs_bg,
            render_groups,
        );
        dispatch(
            "farthest mip1",
            &self.farthest_mip1,
            &reduce_bg,
            half_groups,
        );
        dispatch(
            "prepare reactivity",
            &self.reactivity,
            &reactivity_bg,
            render_groups,
        );
        dispatch(
            "luma instability",
            &self.instability,
            &instability_bg,
            render_groups,
        );
        dispatch(
            "accumulate",
            &self.accumulate,
            &accumulate_bg,
            output_groups,
        );

        state.index = target;
        state.reset = false;
        state.frame_index = state.frame_index.saturating_add(1);
        state.prev_jitter = inputs.jitter;
        state.prev_exposure = exposure;

        &t.output.view
    }
}

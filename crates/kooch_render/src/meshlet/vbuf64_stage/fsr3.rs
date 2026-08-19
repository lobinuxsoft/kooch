//! FSR 3.1, transliterated (#481, and it closes step 6).
//!
//! The engine's second temporal upscaler, and the one #481 named from
//! the start. SGSR 2 got built first because it is small and cheap;
//! this one is neither, and it is here because it has the quality —
//! feature locking, reactivity, an exact disocclusion test — that a
//! two-pass upscaler structurally cannot have.
//!
//! # Where the passes came from
//!
//! `Kits/FidelityFX/upscalers/fsr3/` of the AMD FSR SDK 2.3.0, MIT.
//! The schedule in `ffx_fsr3upscaler.cpp:1166` is seven dispatches;
//! this is four of them, and the three that are missing are named in
//! the header of `fsr3_prepare_reactivity.wgsl` along with what their
//! absence costs.
//!
//! | Ours | Theirs |
//! |---|---|
//! | `prepare_inputs` | `pipelinePrepareInputs` |
//! | `farthest_depth_mip1` | one level of `pipelineLumaPyramid` |
//! | `prepare_reactivity` | `pipelinePrepareReactivity` |
//! | `luma_instability` | `pipelineLumaInstability` |
//! | `accumulate` | `pipelineAccumulate` |
//! | — | `pipelineShadingChangePyramid`, `pipelineShadingChange` |
//! | `sharpen.rs` (already shipped, #876) | `pipelineRCAS` |
//!
//! # It has the same oracle SGSR 2 had
//!
//! 🎯 **At a ratio of 1:1 a temporal upscaler IS a TAA.** Run it
//! un-upscaled against the resolve that already ships, on the same
//! frames, and a port that is wrong shows up as a difference from a
//! known-good image rather than as a vague softness. That is what
//! separates "did I port it correctly" from "does the resolution split
//! work", and it is the answer to the risk #481 wrote down — that a
//! transliteration has nothing to diff against.
//!
//! # Licence
//!
//! MIT, Advanced Micro Devices. The copyright header stays in every
//! ported file and the full text is in `NOTICE`. MIT asks for
//! attribution and nothing else.

mod targets;
#[cfg(test)]
mod tests;

use bytemuck::{Pod, Zeroable};

use targets::Targets;

use super::sgsr2::UpscaleInputs;
use crate::meshlet::deferred::HDR_COLOR_FORMAT;

const COMMON_SOURCE: &str = include_str!("../../../shaders/fsr3_common.wgsl");
const PREPARE_INPUTS_SOURCE: &str = include_str!("../../../shaders/fsr3_prepare_inputs.wgsl");
const REDUCE_SOURCE: &str = include_str!("../../../shaders/fsr3_reduce.wgsl");
const REACTIVITY_SOURCE: &str = include_str!("../../../shaders/fsr3_prepare_reactivity.wgsl");
const INSTABILITY_SOURCE: &str = include_str!("../../../shaders/fsr3_luma_instability.wgsl");
const ACCUMULATE_SOURCE: &str = include_str!("../../../shaders/fsr3_accumulate.wgsl");

/// WGSL has no `#include`, so each pass is compiled as the shared half
/// followed by its own. Keeping them as separate files rather than one
/// module with five entry points matters for the bind groups: an entry
/// point may only declare bindings its own pass uses.
fn source(pass: &str) -> String {
    format!("{COMMON_SOURCE}\n{pass}")
}

/// The resolved image, and next frame's history. Same format as the
/// resolve's, so the tonemap downstream cannot tell which technique
/// produced what it reads.
pub const FSR3_OUTPUT_FORMAT: wgpu::TextureFormat = HDR_COLOR_FORMAT;

/// FSR's own workgroup size for every pass in this schedule.
const GROUP: u32 = 8;

fn groups(size: (u32, u32)) -> (u32, u32) {
    (size.0.max(1).div_ceil(GROUP), size.1.max(1).div_ceil(GROUP))
}

/// Laid out by hand to 96 bytes so that the WGSL declaration and this
/// one cannot drift: every `vec2` is 8-aligned, the scalars fill the
/// tail, and the padding takes the size to a multiple of 16 because
/// that is what the uniform address space requires.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct Fsr3Ubo {
    render_size: [f32; 2],
    output_size: [f32; 2],
    render_size_rcp: [f32; 2],
    output_size_rcp: [f32; 2],
    jitter: [f32; 2],
    prev_jitter: [f32; 2],
    downscale: [f32; 2],
    near: f32,
    exposure: f32,
    reset: f32,
    frame_index: u32,
    delta_pre_exposure: f32,
    jitter_sequence_length: f32,
    debug: u32,
    _pad: [f32; 3],
}

/// Which half of each ping-pong holds the previous frame, and whether
/// there is anything in it. Behind a lock for the same reason the
/// resolve's is: the render chain is on `&self`.
struct History {
    index: usize,
    reset: bool,
    frame_index: u32,
    prev_jitter: glam::Vec2,
    prev_exposure: f32,
    /// The last debug stage this instance logged, so the line below
    /// appears once per change instead of once per frame.
    logged_stage: Option<u32>,
}

const COMPUTE: wgpu::ShaderStages = wgpu::ShaderStages::COMPUTE;

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn sampled(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Anything the engine reads through the linear sampler. `Rgba16Float`
/// is filterable; `R32Float` is not, which is why the two are separate
/// helpers rather than one with a flag nobody would read.
fn filterable(binding: u32) -> wgpu::BindGroupLayoutEntry {
    sampled(binding, wgpu::TextureSampleType::Float { filterable: true })
}

fn unfilterable(binding: u32) -> wgpu::BindGroupLayoutEntry {
    sampled(
        binding,
        wgpu::TextureSampleType::Float { filterable: false },
    )
}

fn storage(
    binding: u32,
    format: wgpu::TextureFormat,
    access: wgpu::StorageTextureAccess,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn write_hdr(binding: u32) -> wgpu::BindGroupLayoutEntry {
    storage(
        binding,
        HDR_COLOR_FORMAT,
        wgpu::StorageTextureAccess::WriteOnly,
    )
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn layout(
    device: &wgpu::Device,
    label: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries,
    })
}

fn pipeline(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    entry: &str,
    bgl: &wgpu::BindGroupLayout,
) -> wgpu::ComputePipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(bgl)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        module,
        entry_point: Some(entry),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

pub(super) struct Fsr3 {
    prepare_inputs: wgpu::ComputePipeline,
    clear_reconstructed: wgpu::ComputePipeline,
    prepare_inputs_bgl: wgpu::BindGroupLayout,

    farthest_mip1: wgpu::ComputePipeline,
    clear_new_locks: wgpu::ComputePipeline,
    reduce_bgl: wgpu::BindGroupLayout,

    reactivity: wgpu::ComputePipeline,
    reactivity_bgl: wgpu::BindGroupLayout,

    instability: wgpu::ComputePipeline,
    instability_bgl: wgpu::BindGroupLayout,

    accumulate: wgpu::ComputePipeline,
    accumulate_bgl: wgpu::BindGroupLayout,

    ubo: wgpu::Buffer,
    linear: wgpu::Sampler,
    targets: Targets,
    render_size: (u32, u32),
    output_size: (u32, u32),
    state: std::sync::Mutex<History>,
}

impl Fsr3 {
    pub(super) fn new(device: &wgpu::Device, render: (u32, u32), output: (u32, u32)) -> Self {
        let module = |label: &str, pass: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source(pass).into()),
            })
        };

        let inputs_module = module("fsr3_prepare_inputs", PREPARE_INPUTS_SOURCE);
        let reduce_module = module("fsr3_reduce", REDUCE_SOURCE);
        let reactivity_module = module("fsr3_prepare_reactivity", REACTIVITY_SOURCE);
        let instability_module = module("fsr3_luma_instability", INSTABILITY_SOURCE);
        let accumulate_module = module("fsr3_accumulate", ACCUMULATE_SOURCE);

        let prepare_inputs_bgl = layout(
            device,
            "fsr3_prepare_inputs_bgl",
            &[
                uniform(0),
                sampled(1, wgpu::TextureSampleType::Depth),
                filterable(2),
                filterable(3),
                storage(
                    4,
                    wgpu::TextureFormat::R32Uint,
                    wgpu::StorageTextureAccess::Atomic,
                ),
                write_hdr(5),
                storage(
                    6,
                    wgpu::TextureFormat::R32Float,
                    wgpu::StorageTextureAccess::WriteOnly,
                ),
                write_hdr(7),
            ],
        );
        let reduce_bgl = layout(
            device,
            "fsr3_reduce_bgl",
            &[
                uniform(0),
                filterable(1),
                write_hdr(2),
                storage(
                    3,
                    wgpu::TextureFormat::R32Float,
                    wgpu::StorageTextureAccess::WriteOnly,
                ),
            ],
        );
        let reactivity_bgl = layout(
            device,
            "fsr3_reactivity_bgl",
            &[
                uniform(0),
                filterable(1),
                unfilterable(2),
                sampled(3, wgpu::TextureSampleType::Uint),
                filterable(4),
                filterable(5),
                sampler_entry(6),
                write_hdr(7),
                write_hdr(8),
                storage(
                    9,
                    wgpu::TextureFormat::R32Float,
                    wgpu::StorageTextureAccess::WriteOnly,
                ),
            ],
        );
        let instability_bgl = layout(
            device,
            "fsr3_instability_bgl",
            &[
                uniform(0),
                filterable(1),
                filterable(2),
                filterable(3),
                filterable(4),
                sampler_entry(5),
                write_hdr(6),
                write_hdr(7),
            ],
        );
        let accumulate_bgl = layout(
            device,
            "fsr3_accumulate_bgl",
            &[
                uniform(0),
                filterable(1),
                filterable(2),
                filterable(3),
                filterable(4),
                filterable(5),
                filterable(6),
                sampler_entry(7),
                storage(
                    8,
                    wgpu::TextureFormat::R32Float,
                    wgpu::StorageTextureAccess::ReadWrite,
                ),
                write_hdr(9),
                unfilterable(10),
                storage(
                    11,
                    wgpu::TextureFormat::R32Float,
                    wgpu::StorageTextureAccess::WriteOnly,
                ),
            ],
        );

        Self {
            prepare_inputs: pipeline(
                device,
                "fsr3_prepare_inputs",
                &inputs_module,
                "prepare_inputs",
                &prepare_inputs_bgl,
            ),
            clear_reconstructed: pipeline(
                device,
                "fsr3_clear_reconstructed_depth",
                &inputs_module,
                "clear_reconstructed_depth",
                &prepare_inputs_bgl,
            ),
            prepare_inputs_bgl,
            farthest_mip1: pipeline(
                device,
                "fsr3_farthest_depth_mip1",
                &reduce_module,
                "farthest_depth_mip1",
                &reduce_bgl,
            ),
            clear_new_locks: pipeline(
                device,
                "fsr3_clear_new_locks",
                &reduce_module,
                "clear_new_locks",
                &reduce_bgl,
            ),
            reduce_bgl,
            reactivity: pipeline(
                device,
                "fsr3_prepare_reactivity",
                &reactivity_module,
                "prepare_reactivity",
                &reactivity_bgl,
            ),
            reactivity_bgl,
            instability: pipeline(
                device,
                "fsr3_luma_instability",
                &instability_module,
                "luma_instability",
                &instability_bgl,
            ),
            instability_bgl,
            accumulate: pipeline(
                device,
                "fsr3_accumulate",
                &accumulate_module,
                "accumulate",
                &accumulate_bgl,
            ),
            accumulate_bgl,
            ubo: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fsr3_ubo"),
                size: std::mem::size_of::<Fsr3Ubo>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            linear: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("fsr3_linear"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            }),
            targets: Targets::new(device, render, output),
            render_size: render,
            output_size: output,
            state: std::sync::Mutex::new(History {
                index: 0,
                reset: true,
                frame_index: 0,
                prev_jitter: glam::Vec2::ZERO,
                prev_exposure: 1.0,
                logged_stage: None,
            }),
        }
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, render: (u32, u32), output: (u32, u32)) {
        if (render, output) == (self.render_size, self.output_size)
            || render.0 == 0
            || render.1 == 0
        {
            return;
        }
        self.targets = Targets::new(device, render, output);
        self.render_size = render;
        self.output_size = output;
        // Every history in here is at the old grid, so none of it means
        // anything at the new one.
        let mut state = self.state.lock().expect("fsr3 history lock");
        state.reset = true;
        state.frame_index = 0;
    }

    /// The image the tonemap reads. Valid only after [`Self::draw`] has
    /// run this frame; before the first one it is the cleared half of
    /// the pair, which is black rather than undefined.
    pub(super) fn resolved_texture(&self) -> &wgpu::Texture {
        let state = self.state.lock().expect("fsr3 history lock");
        self.targets.history.texture(state.index)
    }

    /// Marks the next frame as having no usable history — a camera cut,
    /// a teleport, anything that makes reprojection a lie.
    pub(super) fn reset(&self) {
        self.state.lock().expect("fsr3 history lock").reset = true;
    }

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
    pub(super) fn draw(
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
                    resource: texture(t.lock.view(previous)),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: texture(t.lock.view(target)),
                },
            ],
        });

        let render_groups = groups(self.render_size);
        let half_groups = groups((self.render_size.0 / 2, self.render_size.1 / 2));
        let output_groups = groups(self.output_size);

        let mut dispatch = |label: &str,
                            pipeline: &wgpu::ComputePipeline,
                            bind_group: &wgpu::BindGroup,
                            count: (u32, u32)| {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.dispatch_workgroups(count.0, count.1, 1);
        };

        // The scatter only writes where something reprojects to, so
        // anything it misses would keep the previous frame's depth and
        // report a false occlusion.
        dispatch(
            "fsr3_clear_reconstructed_depth",
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
                "fsr3_clear_new_locks",
                &self.clear_new_locks,
                &reduce_bg,
                output_groups,
            );
        }
        dispatch(
            "fsr3_prepare_inputs",
            &self.prepare_inputs,
            &inputs_bg,
            render_groups,
        );
        dispatch(
            "fsr3_farthest_depth_mip1",
            &self.farthest_mip1,
            &reduce_bg,
            half_groups,
        );
        dispatch(
            "fsr3_prepare_reactivity",
            &self.reactivity,
            &reactivity_bg,
            render_groups,
        );
        dispatch(
            "fsr3_luma_instability",
            &self.instability,
            &instability_bg,
            render_groups,
        );
        dispatch(
            "fsr3_accumulate",
            &self.accumulate,
            &accumulate_bg,
            output_groups,
        );

        state.index = target;
        state.reset = false;
        state.frame_index = state.frame_index.saturating_add(1);
        state.prev_jitter = inputs.jitter;
        state.prev_exposure = exposure;

        t.history.view(target)
    }
}

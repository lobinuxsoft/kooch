//! Snapdragon Game Super Resolution 2, transliterated (#481, step 4).
//!
//! The engine's own temporal upscaler, ported rather than invented. The
//! reasoning behind picking this one over FSR 3.1 is in the header of
//! `sgsr2_convert.wgsl`; the short version is that FSR has the quality
//! and SGSR has the cost, and cost is what this engine is short of —
//! 40.7 ms against a 13.9 ms budget.
//!
//! # 🎯 It has an oracle, which the plan said this work would not
//!
//! The risk written into #481 was that a transliteration has nothing to
//! diff against, so it is validated by eye and degenerates into "it
//! looks wrong and I do not know why".
//!
//! That is avoidable: **at a ratio of 1:1 this IS a TAA**. Run it
//! un-upscaled against the resolve that already ships, on the same
//! frames, and a port that is wrong shows up as a difference from a
//! known-good image rather than as a vague softness. It separates "did I
//! port it correctly" from "does the resolution split work", which are
//! the two risks the plan had bundled into one.
//!
//! # Licence
//!
//! BSD 3-Clause, Qualcomm Innovation Center. The copyright header stays
//! in every ported file and the full text is in `NOTICE`. The third
//! clause also forbids using their name to endorse this — so: **this is
//! not a Qualcomm product and Qualcomm has not endorsed it.**

// ⚠️ Unused until the upscale pass lands, and allowed rather than left
// to warn: these constants are compiled into every project that builds
// on this engine, and a warning in someone else's build is noise they
// cannot act on. The `#[allow]` comes off with the pass that uses them.
#![allow(dead_code)]

const CONVERT_SOURCE: &str = include_str!("../../../shaders/sgsr2_convert.wgsl");
const UPSCALE_SOURCE: &str = include_str!("../../../shaders/sgsr2_upscale.wgsl");

/// What the convert pass writes and the upscale pass reads.
///
/// `xy` dilated motion in UV, `z` the depth-clip factor, `w` unused.
/// Half precision: the motion is already `Rg16Float` upstream of this
/// and the clip factor is a `[0, 1]` weight, so nothing here has a range
/// that fp16 cannot describe.
pub const SGSR2_CONVERT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Upstream's `Params.cameraFovAngleHor`.
///
/// 🔴 Verified rather than guessed, which matters because it scales a
/// tuned constant. Qualcomm's public repository ships **only the
/// shaders** — no host code — so the value was recovered from a
/// community Unity port (`whitecostume/SGSR2_Unity`), which computes
/// `tan(fov_vertical / 2) * aspect`. That is `tan(fov_horizontal / 2)`,
/// and it agrees with the dimensional analysis of the expression it
/// feeds: two independent routes to the same number.
pub fn fov_k(fov_vertical: f32, aspect: f32) -> f32 {
    (fov_vertical * 0.5).tan() * aspect
}

/// Upstream's `Params.scaleRatio`.
///
/// `.x` is the linear upscale ratio and becomes the Lanczos kernel's
/// bias. `.y` is the **cube of the area ratio, capped at 20** — their
/// number, and the cap is theirs too. It widens the variance box as the
/// upscale gets more aggressive, because a box built from fewer input
/// samples is a worse estimate of the neighbourhood and clamping the
/// history to it too tightly is what makes an upscaler flicker.
///
/// At 1:1 it is `(1, 1)`, which is the identity this is validated at.
pub fn scale_ratio(render: (u32, u32), display: (u32, u32)) -> [f32; 2] {
    let linear = display.0.max(1) as f32 / render.0.max(1) as f32;
    let area = (display.0.max(1) as f32 * display.1.max(1) as f32)
        / (render.0.max(1) as f32 * render.1.max(1) as f32);
    [linear, (area * area * area).min(20.0)]
}

/// Upstream's `Params.minLerpContribution`, unchanged at their default.
///
/// How much of the history survives when it lands outside the
/// neighbourhood box **and the pixel is not moving**. A moving pixel
/// gets zero instead — see the upscale shader.
pub const MIN_LERP_CONTRIBUTION: f32 = 0.3;

#[cfg(test)]
mod tests;

use bytemuck::{Pod, Zeroable};

use crate::meshlet::deferred::HDR_COLOR_FORMAT;

/// The resolved image, and next frame's history. Same format as the
/// resolve's, so the tonemap downstream cannot tell which technique
/// produced what it reads — which is what makes them swappable.
pub const SGSR2_OUTPUT_FORMAT: wgpu::TextureFormat = HDR_COLOR_FORMAT;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct ConvertUbo {
    render_size: [f32; 2],
    render_size_rcp: [f32; 2],
    fov_k: f32,
    _pad: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct UpscaleUbo {
    render_size: [f32; 2],
    output_size: [f32; 2],
    render_size_rcp: [f32; 2],
    output_size_rcp: [f32; 2],
    jitter_offset: [f32; 2],
    scale_ratio: [f32; 2],
    min_lerp: f32,
    reset: f32,
    exposure: f32,
    _pad: f32,
}

/// Which half of the output pair holds the previous frame, and whether
/// there is anything in it. Behind a lock for the same reason the
/// resolve's is: the render chain is on `&self`.
struct History {
    index: usize,
    reset: bool,
}

/// Everything this technique consumes, which is FSR 3.1's contract kept
/// deliberately (#536).
///
/// A struct rather than eight arguments because the NEXT backend takes
/// the same six things, and a signature both can satisfy is what makes
/// adding one a day's work.
pub(super) struct UpscaleInputs<'a> {
    /// The jittered frame, at render resolution.
    pub color: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub motion: &'a wgpu::TextureView,
    /// This frame's sub-pixel offset, in RENDER pixels.
    pub jitter: glam::Vec2,
    pub exposure: f32,
    /// `tan(fov_vertical / 2) * aspect`; see [`fov_k`].
    pub fov_k: f32,
    /// The near plane. Under this engine's infinite reversed-Z
    /// projection it is the WHOLE depth transform — `view_z = near / d`
    /// — which is what FSR 3.1 needs to express its thresholds in
    /// metres. SGSR 2 does not read it.
    pub near: f32,
    /// How many sub-pixel offsets the jitter sequence cycles through.
    /// FSR decays a feature lock over exactly one pass of it.
    pub jitter_phases: f32,
    /// Which intermediate to write instead of the image, or 0. Comes
    /// from the editor's debug dropdown; SGSR 2 has no intermediates
    /// worth a legend and ignores it.
    pub debug_stage: u32,
}

pub(super) struct Sgsr2 {
    convert_pipeline: wgpu::RenderPipeline,
    convert_bgl: wgpu::BindGroupLayout,
    convert_ubo: wgpu::Buffer,
    upscale_pipeline: wgpu::RenderPipeline,
    upscale_bgl: wgpu::BindGroupLayout,
    upscale_ubo: wgpu::Buffer,
    point: wgpu::Sampler,
    linear: wgpu::Sampler,
    /// `xy` dilated motion, `z` depth clip. Render resolution — it is
    /// consumed at the input grid, not the output one.
    convert_view: wgpu::TextureView,
    output: [wgpu::Texture; 2],
    output_views: [wgpu::TextureView; 2],
    render_size: (u32, u32),
    output_size: (u32, u32),
    state: std::sync::Mutex<History>,
}

impl Sgsr2 {
    pub(super) fn new(device: &wgpu::Device, render: (u32, u32), output: (u32, u32)) -> Self {
        let convert_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sgsr2_convert"),
            source: wgpu::ShaderSource::Wgsl(CONVERT_SOURCE.into()),
        });
        let upscale_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sgsr2_upscale"),
            source: wgpu::ShaderSource::Wgsl(UPSCALE_SOURCE.into()),
        });

        let sampled = |binding: u32, filterable: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry =
            |binding: u32, ty: wgpu::SamplerBindingType| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(ty),
                count: None,
            };
        let uniform = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let convert_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sgsr2_convert_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                sampled(1, false),
                sampler_entry(2, wgpu::SamplerBindingType::NonFiltering),
                uniform(3),
            ],
        });

        let upscale_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sgsr2_upscale_bgl"),
            entries: &[
                uniform(0),
                // 🔴 The one filtered read. The history is sampled
                // between texels by construction — that is what
                // reprojection means — and a point tap there returns the
                // texel the history was written to rather than the one
                // this pixel came from.
                sampled(1, true),
                sampled(2, false),
                sampled(3, false),
                sampler_entry(4, wgpu::SamplerBindingType::Filtering),
                sampler_entry(5, wgpu::SamplerBindingType::NonFiltering),
            ],
        });

        let pipeline = |label: &str,
                        bgl: &wgpu::BindGroupLayout,
                        module: &wgpu::ShaderModule,
                        vs: &str,
                        fs: &str,
                        format: wgpu::TextureFormat| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(bgl)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vs),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs),
                    targets: &[Some(format.into())],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let convert_pipeline = pipeline(
            "sgsr2_convert_pipeline",
            &convert_bgl,
            &convert_module,
            "vs_convert",
            "fs_convert",
            SGSR2_CONVERT_FORMAT,
        );
        let upscale_pipeline = pipeline(
            "sgsr2_upscale_pipeline",
            &upscale_bgl,
            &upscale_module,
            "vs_upscale",
            "fs_upscale",
            SGSR2_OUTPUT_FORMAT,
        );

        let ubo = |label: &str, size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };

        let point = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sgsr2_point"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sgsr2_linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let (convert_view, output_textures, output_views) = create_targets(device, render, output);
        Self {
            convert_pipeline,
            convert_bgl,
            convert_ubo: ubo(
                "sgsr2_convert_ubo",
                std::mem::size_of::<ConvertUbo>() as u64,
            ),
            upscale_pipeline,
            upscale_bgl,
            upscale_ubo: ubo(
                "sgsr2_upscale_ubo",
                std::mem::size_of::<UpscaleUbo>() as u64,
            ),
            point,
            linear,
            convert_view,
            output: output_textures,
            output_views,
            render_size: render,
            output_size: output,
            state: std::sync::Mutex::new(History {
                index: 0,
                reset: true,
            }),
        }
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, render: (u32, u32), output: (u32, u32)) {
        let (convert_view, textures, views) = create_targets(device, render, output);
        self.convert_view = convert_view;
        self.output = textures;
        self.output_views = views;
        self.render_size = render;
        self.output_size = output;
        let mut state = self.state.lock().expect("sgsr2 history lock");
        state.index = 0;
        state.reset = true;
    }

    /// The most recent resolve, for a test to read back.
    pub(super) fn resolved_texture(&self) -> &wgpu::Texture {
        let index = self.state.lock().expect("sgsr2 history lock").index;
        &self.output[index]
    }

    /// Runs both passes and returns the view the tonemap should read.
    pub(super) fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        inputs: UpscaleInputs<'_>,
    ) -> &wgpu::TextureView {
        let mut state = self.state.lock().expect("sgsr2 history lock");
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

        queue.write_buffer(
            &self.convert_ubo,
            0,
            bytemuck::bytes_of(&ConvertUbo {
                render_size: [render.0, render.1],
                render_size_rcp: [1.0 / render.0, 1.0 / render.1],
                fov_k: inputs.fov_k,
                _pad: 0.0,
            }),
        );
        queue.write_buffer(
            &self.upscale_ubo,
            0,
            bytemuck::bytes_of(&UpscaleUbo {
                render_size: [render.0, render.1],
                output_size: [out.0, out.1],
                render_size_rcp: [1.0 / render.0, 1.0 / render.1],
                output_size_rcp: [1.0 / out.0, 1.0 / out.1],
                jitter_offset: [inputs.jitter.x, inputs.jitter.y],
                scale_ratio: scale_ratio(self.render_size, self.output_size),
                min_lerp: MIN_LERP_CONTRIBUTION,
                reset: f32::from(u8::from(state.reset)),
                exposure: inputs.exposure,
                _pad: 0.0,
            }),
        );

        let convert_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sgsr2_convert_bg"),
            layout: &self.convert_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(inputs.depth),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(inputs.motion),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.point),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.convert_ubo.as_entire_binding(),
                },
            ],
        });
        let upscale_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sgsr2_upscale_bg"),
            layout: &self.upscale_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.upscale_ubo.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.output_views[previous]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.convert_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(inputs.color),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.linear),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.point),
                },
            ],
        });

        let mut pass = |label: &'static str,
                        view: &wgpu::TextureView,
                        pipeline: &wgpu::RenderPipeline,
                        bind_group: &wgpu::BindGroup| {
            let mut p = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    // Every pixel is written, so the load discards.
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(pipeline);
            p.set_bind_group(0, bind_group, &[]);
            p.draw(0..3, 0..1);
        };

        pass(
            "sgsr2_convert_pass",
            &self.convert_view,
            &self.convert_pipeline,
            &convert_bg,
        );
        pass(
            "sgsr2_upscale_pass",
            &self.output_views[target],
            &self.upscale_pipeline,
            &upscale_bg,
        );

        state.index = target;
        state.reset = false;
        &self.output_views[target]
    }
}

fn create_targets(
    device: &wgpu::Device,
    render: (u32, u32),
    output: (u32, u32),
) -> (
    wgpu::TextureView,
    [wgpu::Texture; 2],
    [wgpu::TextureView; 2],
) {
    let make = |label: &str, size: (u32, u32), format: wgpu::TextureFormat| {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            // COPY_SRC so a test can read the resolve back.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    };
    let (_convert, convert_view) = make("sgsr2_convert", render, SGSR2_CONVERT_FORMAT);
    let (o0, v0) = make("sgsr2_output_0", output, SGSR2_OUTPUT_FORMAT);
    let (o1, v1) = make("sgsr2_output_1", output, SGSR2_OUTPUT_FORMAT);
    (convert_view, [o0, o1], [v0, v1])
}

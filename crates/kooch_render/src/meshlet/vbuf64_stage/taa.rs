//! Temporal anti-aliasing (#481).
//!
//! The consumer the motion vectors and the sub-pixel jitter were built
//! for. It sits between the shading's HDR target and the tonemap,
//! because blending frames is only meaningful on linear radiance — which
//! is the reason the tonemap became a pass of its own (#732).
//!
//! # Two ping-pong pairs, and why the resolve is also the history
//!
//! Upstream writes two full-resolution `Rgba16Float` attachments: the
//! resolved image, and a private history in a range-compressed space.
//! Here the compression happens on read, so the resolved image *is* the
//! history and there is one colour texture rather than two. That is not
//! only cheaper — it is measurably more accurate, because fp16
//! quantisation of a compressed value is amplified by the inverse
//! operator. The numbers are in the header of `taa.wgsl`.
//!
//! What could not be folded in is the confidence counter. Upstream keeps
//! it in the history's alpha, which is free there because its history is
//! private; here the same texture reaches the tonemap, where alpha means
//! coverage. It gets its own `R16Float` pair — two bytes a pixel to keep
//! the two meanings apart, against eight to duplicate the colour.

use bytemuck::{Pod, Zeroable};

use crate::meshlet::deferred::HDR_COLOR_FORMAT;

const SHADER_SOURCE: &str = include_str!("../../../shaders/taa.wgsl");

/// The resolved image and next frame's history, one and the same.
pub const TAA_COLOR_FORMAT: wgpu::TextureFormat = HDR_COLOR_FORMAT;

/// How many still frames a pixel has accumulated. One channel, half
/// precision: the counter saturates the blend rate at 1/64 and never
/// needs to represent more than a few hundred.
pub const TAA_CONFIDENCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct TaaUbo {
    reset: u32,
    _pad: [u32; 3],
}

/// Ping-pong state, behind a lock for the same reason the motion pass's
/// previous matrix is: the whole render chain is on `&self` and the
/// stage lives in `Resources`, which requires `Sync`.
struct History {
    /// Which half of each pair holds the PREVIOUS frame. The pass writes
    /// the other one.
    index: usize,
    /// Set on the first frame and after every resize. There is nothing
    /// to reproject from, and blending against a cleared texture is a
    /// black frame that fades in over sixty of them.
    reset: bool,
}

pub(super) struct Taa {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    ubo: wgpu::Buffer,
    nearest: wgpu::Sampler,
    linear: wgpu::Sampler,
    color: [wgpu::Texture; 2],
    color_views: [wgpu::TextureView; 2],
    confidence: [wgpu::Texture; 2],
    confidence_views: [wgpu::TextureView; 2],
    state: std::sync::Mutex<History>,
}

impl Taa {
    pub(super) fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("taa_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let texture = |binding: u32, filterable: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler = |binding: u32, ty: wgpu::SamplerBindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(ty),
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("taa_bgl"),
            entries: &[
                texture(0, false),
                // 🔴 The one texture read with a filtering sampler. The
                // five-tap Catmull-Rom reprojection lands between texels
                // by construction — that is what reprojection means —
                // and a nearest tap there is the sample the history was
                // written for, not the one this pixel came from.
                texture(1, true),
                texture(2, false),
                texture(3, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                sampler(5, wgpu::SamplerBindingType::NonFiltering),
                sampler(6, wgpu::SamplerBindingType::Filtering),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("taa_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("taa_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_taa"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_taa"),
                targets: &[
                    Some(TAA_COLOR_FORMAT.into()),
                    Some(TAA_CONFIDENCE_FORMAT.into()),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let ubo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("taa_ubo"),
            size: std::mem::size_of::<TaaUbo>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let nearest = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("taa_nearest"),
            // Clamped, and the shader rejects an off-screen reprojection
            // outright — the clamp is what stops a border texel being
            // read at all, the rejection is what stops it being trusted.
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("taa_linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let (color, color_views, confidence, confidence_views) = create_targets(device, size);
        Self {
            pipeline,
            bgl,
            ubo,
            nearest,
            linear,
            color,
            color_views,
            confidence,
            confidence_views,
            state: std::sync::Mutex::new(History {
                index: 0,
                reset: true,
            }),
        }
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let (color, color_views, confidence, confidence_views) = create_targets(device, size);
        self.color = color;
        self.color_views = color_views;
        self.confidence = confidence;
        self.confidence_views = confidence_views;
        let mut state = self.state.lock().expect("taa history lock");
        state.index = 0;
        state.reset = true;
    }

    /// The texture holding the most recent resolve. What a test reads;
    /// the tonemap takes the view this pass returns instead.
    pub(super) fn resolved_texture(&self) -> &wgpu::Texture {
        let index = self.state.lock().expect("taa history lock").index;
        &self.color[index]
    }

    /// Resolves `current` against the history and returns the view the
    /// tonemap should read.
    ///
    /// ⚠️ `current` must be the JITTERED frame and `motion` must have
    /// been produced from unjittered matrices. Those are the two halves
    /// of the contract, and both fail quietly: no jitter is a resolve
    /// that averages a frame with itself, and jittered motion vectors
    /// cancel the jitter back out. Between them they describe a TAA that
    /// runs, costs and does nothing.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        current: &wgpu::TextureView,
        motion: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) -> &wgpu::TextureView {
        let mut state = self.state.lock().expect("taa history lock");
        let previous = state.index;
        let target = 1 - previous;

        queue.write_buffer(
            &self.ubo,
            0,
            bytemuck::bytes_of(&TaaUbo {
                reset: u32::from(state.reset),
                _pad: [0; 3],
            }),
        );

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("taa_bind_group"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(current),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.color_views[previous]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.confidence_views[previous]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(motion),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.nearest),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&self.linear),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: self.ubo.as_entire_binding(),
                },
            ],
        });

        {
            fn attachment(view: &wgpu::TextureView) -> Option<wgpu::RenderPassColorAttachment<'_>> {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    // Every pixel is written, so nothing is preserved and
                    // the load is a discard rather than a read.
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("taa_pass"),
                color_attachments: &[
                    attachment(&self.color_views[target]),
                    attachment(&self.confidence_views[target]),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        state.index = target;
        state.reset = false;
        &self.color_views[target]
    }
}

type Targets = (
    [wgpu::Texture; 2],
    [wgpu::TextureView; 2],
    [wgpu::Texture; 2],
    [wgpu::TextureView; 2],
);

fn create_targets(device: &wgpu::Device, size: (u32, u32)) -> Targets {
    let make = |label: &str, format: wgpu::TextureFormat| {
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
            // COPY_SRC so a test can read the resolve back; the tonemap
            // samples rather than copies.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    };
    let (c0, v0) = make("taa_history_0", TAA_COLOR_FORMAT);
    let (c1, v1) = make("taa_history_1", TAA_COLOR_FORMAT);
    let (f0, w0) = make("taa_confidence_0", TAA_CONFIDENCE_FORMAT);
    let (f1, w1) = make("taa_confidence_1", TAA_CONFIDENCE_FORMAT);
    ([c0, c1], [v0, v1], [f0, f1], [w0, w1])
}

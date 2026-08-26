//! The pass between HDR radiance and the image (#732 phase 1).
//!
//! The compute shading path writes linear radiance into an
//! [`HDR_COLOR_FORMAT`] texture and this resolves it onto the caller's
//! `Rgba8Unorm` view. Temporal anti-aliasing lands between the two:
//! blending this frame with the last is only meaningful before the
//! curve, which is why the tonemap had to leave the shading shader at
//! all.
//!
//! The fragment shading path is untouched and still tonemaps inline.
//! That is not an oversight — it has no HDR target, will never carry a
//! temporal history, and `compute_shading_parity` compares the two
//! **after** both have reached the same `Rgba8Unorm` view, so the fork
//! is invisible to it.

use bytemuck::{Pod, Zeroable};

use crate::meshlet::deferred::{DEFERRED_COLOR_FORMAT, HDR_COLOR_FORMAT};

const SHADER_SOURCE: &str = include_str!("../../../shaders/tonemap.wgsl");

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct TonemapUbo {
    enabled: u32,
    exposure: f32,
    /// Source-over-target pixel ratio. `1.0` when the source already
    /// matches the target; the render-resolution ratio when a debug
    /// view skipped the upscaler, so the region the renderer actually
    /// wrote stretches to fill the view instead of sitting in a
    /// corner at half size.
    scale: [f32; 2],
}

/// Owns the HDR target the shading writes and the pipeline that resolves
/// it.
pub(super) struct Tonemap {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    ubo: wgpu::Buffer,
    hdr: wgpu::Texture,
    hdr_view: wgpu::TextureView,
}

impl Tonemap {
    pub(super) fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tonemap_shader"),
            // 🔴 The operator concatenated, not copied. See `INTI_TONEMAP`.
            source: wgpu::ShaderSource::Wgsl(
                format!("{}{}", kooch_lighting::INTI_TONEMAP, SHADER_SOURCE).into(),
            ),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tonemap_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // Not filterable: the pass reads texel for texel
                        // and `Rgba16Float` filtering is a feature on
                        // some adapters rather than a guarantee.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
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
            label: Some("tonemap_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tonemap_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(DEFERRED_COLOR_FORMAT.into())],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let ubo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tonemap_ubo"),
            size: std::mem::size_of::<TonemapUbo>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (hdr, hdr_view) = create_hdr(device, size);
        Self {
            pipeline,
            bgl,
            ubo,
            hdr,
            hdr_view,
        }
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let (hdr, hdr_view) = create_hdr(device, size);
        self.hdr = hdr;
        self.hdr_view = hdr_view;
    }

    /// Where the shading writes at full rate, and where the upsample
    /// draws at a reduced one.
    pub(super) fn hdr_view(&self) -> &wgpu::TextureView {
        &self.hdr_view
    }

    /// Resolves `source` onto `target`.
    ///
    /// `source` is this stage's own HDR target most of the time and the
    /// temporal resolve's output when TAA is on (#481) — the pass reads
    /// whichever texture last held linear radiance, which is why it is a
    /// parameter rather than the field beside it.
    ///
    /// `enabled` is false for the debug views, which produce
    /// display-ready false colour: putting a legend through a filmic
    /// curve turns a readable ramp into a washed-out one.
    pub(super) fn draw(
        &self,
        queue: &wgpu::Queue,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
        exposure: f32,
        enabled: bool,
        source_scale: [f32; 2],
    ) {
        queue.write_buffer(
            &self.ubo,
            0,
            bytemuck::bytes_of(&TonemapUbo {
                enabled: u32::from(enabled),
                exposure,
                scale: source_scale,
            }),
        );

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tonemap_bind_group"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.ubo.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("tonemap_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Every pixel is written, so nothing is preserved and
                    // the load is a discard rather than a read.
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn create_hdr(device: &wgpu::Device, size: (u32, u32)) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shading_hdr"),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: HDR_COLOR_FORMAT,
        // STORAGE for the compute shading, RENDER_ATTACHMENT for the
        // upsample, TEXTURE_BINDING for this pass reading it back.
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

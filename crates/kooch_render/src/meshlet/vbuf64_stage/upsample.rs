//! Half-rate lighting back to full resolution (#825).
//!
//! One fullscreen fragment pass, run only when
//! [`ShadingRate::needs_upsample`](super::ShadingRate::needs_upsample).
//! It owns the two half-resolution targets the compute shading pass
//! writes into — the colour and the per-sample surface id — because
//! their size is a function of the rate and nothing else in the stage
//! has a reason to know about it.
//!
//! The pass reads the **full-resolution** visibility buffer for coverage
//! and identity, so the silhouette on screen is the raster's, not the
//! lighting's. See the shader for why the guide is the vbuf and not
//! depth.

use bytemuck::{Pod, Zeroable, bytes_of};

use crate::meshlet::render_stage::create_2d_attachment;

use super::{DEFERRED_COLOR_FORMAT, ShadingRate, VBUF64_FORMAT};

/// Per-sample surface id written by the shading pass, as
/// `visible_slot + 1`. `R32Uint` because `visible_slot` is a 25-bit
/// field and no smaller integer format is guaranteed as a storage
/// texture.
pub(super) const SHADED_ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct UpsampleUbo {
    size: [u32; 2],
    shaded_size: [u32; 2],
}

pub(super) struct ShadingUpsample {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    ubo: wgpu::Buffer,
    color_texture: wgpu::Texture,
    color_view: wgpu::TextureView,
    id_texture: wgpu::Texture,
    id_view: wgpu::TextureView,
    shaded_size: (u32, u32),
}

impl ShadingUpsample {
    /// `size` is the full screen; the half-resolution targets are sized
    /// from it for the largest rate that needs them.
    ///
    /// Allocated unconditionally, and at a cost worth stating: the pair
    /// is 5 bytes per shaded sample, so 1.25 bytes per screen pixel —
    /// 1.1 MB at 1280x720. Allocating it lazily would mean a stall on
    /// the frame the player changes the quality setting, which is the
    /// one frame they are looking at it (#830).
    pub(super) fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shading_upsample_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../shaders/shading_upsample.wgsl").into(),
            ),
        });
        let sampled =
            |binding: u32, sample_type: wgpu::TextureSampleType| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shading_upsample_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::ReadOnly,
                        format: VBUF64_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                sampled(1, wgpu::TextureSampleType::Float { filterable: true }),
                sampled(2, wgpu::TextureSampleType::Uint),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<UpsampleUbo>() as u64,
                        ),
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shading_upsample_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shading_upsample_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_fullscreen"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_upsample"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: DEFERRED_COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let ubo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shading_upsample_ubo"),
            size: std::mem::size_of::<UpsampleUbo>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shaded_size = ShadingRate::Half.target_size(size);
        let (color_texture, color_view) = create_shaded_color(device, shaded_size);
        let (id_texture, id_view) = create_shaded_ids(device, shaded_size);
        Self {
            pipeline,
            bgl,
            ubo,
            color_texture,
            color_view,
            id_texture,
            id_view,
            shaded_size,
        }
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let shaded_size = ShadingRate::Half.target_size(size);
        if shaded_size == self.shaded_size {
            return;
        }
        let (color_texture, color_view) = create_shaded_color(device, shaded_size);
        self.color_texture = color_texture;
        self.color_view = color_view;
        let (id_texture, id_view) = create_shaded_ids(device, shaded_size);
        self.id_texture = id_texture;
        self.id_view = id_view;
        self.shaded_size = shaded_size;
    }

    /// Where the shading pass writes at reduced rate.
    pub(super) fn color_view(&self) -> &wgpu::TextureView {
        &self.color_view
    }

    pub(super) fn id_view(&self) -> &wgpu::TextureView {
        &self.id_view
    }

    /// Clears the id target to 0 — "this sample shaded nothing".
    ///
    /// Must run before the material dispatches, which write only the
    /// samples they own. A stale colour would be invisible behind a
    /// stale id; a stale *id* is what makes the upsample trust it. The
    /// colour target is cleared by the shading pass along with every
    /// other rate, so it is not this function's business.
    pub(super) fn clear_ids(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shading_upsample_id_clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.id_view,
                depth_slice: None,
                resolve_target: None,
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
    }

    /// Records the fullscreen pass that writes the screen from the
    /// shaded samples.
    pub(super) fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        vbuf_view: &wgpu::TextureView,
        color_view: &wgpu::TextureView,
        size: (u32, u32),
    ) {
        queue.write_buffer(
            &self.ubo,
            0,
            bytes_of(&UpsampleUbo {
                size: [size.0, size.1],
                shaded_size: [self.shaded_size.0, self.shaded_size.1],
            }),
        );
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shading_upsample_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(vbuf_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.id_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.ubo.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shading_upsample_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: None,
                // Every pixel is written, background included — the
                // clear is belt and braces for a driver that decides
                // otherwise, and free on a tiler.
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn create_shaded_color(
    device: &wgpu::Device,
    size: (u32, u32),
) -> (wgpu::Texture, wgpu::TextureView) {
    create_2d_attachment(
        device,
        "shading_half_rate_color",
        size,
        DEFERRED_COLOR_FORMAT,
        wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
    )
}

fn create_shaded_ids(
    device: &wgpu::Device,
    size: (u32, u32),
) -> (wgpu::Texture, wgpu::TextureView) {
    create_2d_attachment(
        device,
        "shading_half_rate_ids",
        size,
        SHADED_ID_FORMAT,
        wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
    )
}

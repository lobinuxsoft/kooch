//! Fullscreen debug-visualization pass for the R64 path (#440).

use bytemuck::bytes_of;

use crate::meshlet::dispatcher::MeshletCull;

use super::{DEFERRED_COLOR_FORMAT, VBUF64_FORMAT};

/// What the pass reads: the visibility buffer's own size, the image it covers, and the view. The
/// two sizes differ whenever the render is scaled, and the pass stretches between them.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct DebugUbo {
    size: [u32; 2],
    output: [u32; 2],
    debug_mode: u32,
    _pad: [u32; 3],
}

/// True for debug modes that fully replace shading with a colorized visualization (vs modes that
/// keep the normal look and only change culling or add the reject overlay).
pub(super) fn is_colorize_mode(debug_mode: u32) -> bool {
    crate::meshlet::debug::MeshletDebugMode::all_implemented()
        .iter()
        .find(|mode| mode.as_u32() == debug_mode)
        .is_some_and(|mode| mode.colorizes())
}

pub(super) struct DebugResolve {
    pipeline: wgpu::RenderPipeline,
    /// The same pass blended over a finished image, for the views that draw on top of the scene.
    overlay: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    screen_buffer: wgpu::Buffer,
}

impl DebugResolve {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet_debug_resolve_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../shaders/meshlet_debug_resolve.wgsl").into(),
            ),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_debug_resolve_bgl"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(32),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::ReadOnly,
                        format: wgpu::TextureFormat::R32Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet_debug_resolve_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let build = |label, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_fullscreen"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_debug"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: DEFERRED_COLOR_FORMAT,
                        blend,
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
            })
        };
        let pipeline = build("meshlet_debug_resolve_pipeline", None);
        let overlay = build(
            "meshlet_debug_overlay_pipeline",
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        let screen_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_debug_resolve_screen_ubo"),
            size: std::mem::size_of::<DebugUbo>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            overlay,
            bgl,
            screen_buffer,
        }
    }

    /// Records the fullscreen debug pass, colorizing `color_view` by the
    /// current `debug_mode`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        vbuf_view: &wgpu::TextureView,
        color_view: &wgpu::TextureView,
        density_view: &wgpu::TextureView,
        cull: &MeshletCull,
        screen_size: (u32, u32),
        output_size: (u32, u32),
        debug_mode: u32,
        over: bool,
    ) {
        queue.write_buffer(
            &self.screen_buffer,
            0,
            bytes_of(&DebugUbo {
                size: [screen_size.0, screen_size.1],
                output: [output_size.0.max(1), output_size.1.max(1)],
                debug_mode,
                _pad: [0; 3],
            }),
        );

        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_debug_resolve_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(vbuf_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.screen_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(density_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: cull.visible_meshlets_buffer().as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("meshlet_debug_resolve_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // An overlay keeps the image it draws over; a colorize view owns the frame.
                    load: match over {
                        true => wgpu::LoadOp::Load,
                        false => wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(match over {
            true => &self.overlay,
            false => &self.pipeline,
        });
        pass.set_bind_group(0, &bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests;

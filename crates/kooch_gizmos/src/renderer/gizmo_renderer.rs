use std::mem::size_of;

use kooch_core::resource::Resources;
use wgpu::util::DeviceExt;

use kooch_render::VIEWPORT_DEPTH_FORMAT;

use crate::SHADER_SOURCE;

use super::batch::GizmoBatch;
use super::helpers::{active_camera_view_proj, push_quad};
use super::types::{CameraUniforms, GizmoVertex, INITIAL_VERTEX_CAPACITY};

// ---------------------------------------------------------------------------
// GizmoRenderer
// ---------------------------------------------------------------------------

/// Renders gizmo line segments queued in [`GizmoBatch`] each frame as
/// screen-space quads.
pub struct GizmoRenderer {
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: u64,
}

impl GizmoRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gizmo_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gizmo_camera_buffer"),
            contents: bytemuck::bytes_of(&CameraUniforms::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gizmo_camera_bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gizmo_camera_bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gizmo_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: size_of::<GizmoVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x3, // position
                1 => Float32x3, // color
                2 => Float32x3, // other_position
                3 => Float32,   // side
                4 => Float32,   // thickness
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gizmo_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache,
        });

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gizmo_vertex_buffer"),
            size: INITIAL_VERTEX_CAPACITY * size_of::<GizmoVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            camera_buffer,
            bind_group,
            vertex_buffer,
            vertex_capacity: INITIAL_VERTEX_CAPACITY,
        }
    }

    /// Renders all queued segments. No-ops when the batch is empty or
    /// no active `PerspectiveCamera + GlobalTransform` is found.
    ///
    /// `viewport_size` is the physical-pixel size of the render target.
    /// The vertex shader uses it to convert per-line `thickness` (also
    /// in physical pixels) to NDC offsets.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        resources: &Resources,
        batch: &GizmoBatch,
        viewport_size: (u32, u32),
    ) {
        if batch.lines.is_empty() {
            return;
        }
        if viewport_size.0 == 0 || viewport_size.1 == 0 {
            return;
        }
        let aspect = viewport_size.0 as f32 / viewport_size.1.max(1) as f32;

        let Some(view_proj) = active_camera_view_proj(resources, aspect) else {
            return;
        };

        // Upload camera uniforms.
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&CameraUniforms {
                view_proj: view_proj.to_cols_array_2d(),
                viewport_size: [viewport_size.0 as f32, viewport_size.1 as f32],
                _pad: [0.0; 2],
            }),
        );

        // Build vertex data: 6 vertices per line (two triangles forming
        // a quad in screen-space, expanded by the vertex shader).
        let mut vertices: Vec<GizmoVertex> = Vec::with_capacity(batch.lines.len() * 6);
        for seg in &batch.lines {
            push_quad(seg, &mut vertices);
        }

        // Resize buffer if needed.
        let needed = vertices.len() as u64;
        if needed > self.vertex_capacity {
            let new_cap = needed.next_power_of_two();
            self.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gizmo_vertex_buffer"),
                size: new_cap * size_of::<GizmoVertex>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.vertex_capacity = new_cap;
        }
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gizmo_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..vertices.len() as u32, 0..1);
    }
}

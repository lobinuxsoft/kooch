use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use kooch_core::resource::Resources;
use kooch_ecs::PerspectiveCamera;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::query::Query;
use wgpu::util::DeviceExt;

use kooch_render::VIEWPORT_DEPTH_FORMAT;

use super::SHADER_SOURCE;
use super::batch::{MeshBatch, MeshVertex};

/// Matches the `CameraUniforms` struct in `gizmo_mesh.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
struct CameraUniforms {
    view_proj: [[f32; 4]; 4],
}

/// Initial vertex / index buffer capacities. Grow on demand
/// (next-power-of-two) so steady-state frames don't re-allocate.
const INITIAL_VERTEX_CAPACITY: u64 = 1024;
const INITIAL_INDEX_CAPACITY: u64 = 2048;

/// Renders the queued [`MeshBatch`] each frame as alpha-blended
/// triangles. Always-on-top (depth `Always`, no depth write) — same
/// behavior as the line pass.
pub struct MeshGizmoRenderer {
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    vertex_capacity: u64,
    index_capacity: u64,
}

impl MeshGizmoRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gizmo_mesh_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gizmo_mesh_camera_buffer"),
            contents: bytemuck::bytes_of(&CameraUniforms::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gizmo_mesh_camera_bgl"),
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
            label: Some("gizmo_mesh_camera_bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gizmo_mesh_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: size_of::<MeshVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x3, // position
                1 => Float32x4, // color (rgba)
                2 => Float32x2, // edge_uv
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gizmo_mesh_pipeline"),
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
            label: Some("gizmo_mesh_vertex_buffer"),
            size: INITIAL_VERTEX_CAPACITY * size_of::<MeshVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gizmo_mesh_index_buffer"),
            size: INITIAL_INDEX_CAPACITY * size_of::<u32>() as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            camera_buffer,
            bind_group,
            vertex_buffer,
            index_buffer,
            vertex_capacity: INITIAL_VERTEX_CAPACITY,
            index_capacity: INITIAL_INDEX_CAPACITY,
        }
    }

    /// Renders all queued mesh draws. No-ops on empty batch or no
    /// active camera. Coalesces every `MeshDraw` into a single
    /// vertex/index buffer + draw call (gizmos are tiny so per-draw
    /// dispatch cost would dominate).
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        resources: &Resources,
        batch: &MeshBatch,
        viewport_size: (u32, u32),
    ) {
        if batch.draws.is_empty() {
            return;
        }
        if viewport_size.0 == 0 || viewport_size.1 == 0 {
            return;
        }
        let aspect = viewport_size.0 as f32 / viewport_size.1.max(1) as f32;

        let Some(view_proj) = active_camera_view_proj(resources, aspect) else {
            return;
        };

        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&CameraUniforms {
                view_proj: view_proj.to_cols_array_2d(),
            }),
        );

        // Coalesce all draws into one vertex + index buffer.
        let mut vertices: Vec<MeshVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        for d in &batch.draws {
            let base = vertices.len() as u32;
            vertices.extend_from_slice(&d.vertices);
            indices.extend(d.indices.iter().map(|i| i + base));
        }
        if vertices.is_empty() || indices.is_empty() {
            return;
        }

        if vertices.len() as u64 > self.vertex_capacity {
            let new_cap = (vertices.len() as u64).next_power_of_two();
            self.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gizmo_mesh_vertex_buffer"),
                size: new_cap * size_of::<MeshVertex>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.vertex_capacity = new_cap;
        }
        if indices.len() as u64 > self.index_capacity {
            let new_cap = (indices.len() as u64).next_power_of_two();
            self.index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gizmo_mesh_index_buffer"),
                size: new_cap * size_of::<u32>() as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.index_capacity = new_cap;
        }
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&indices));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gizmo_mesh_pass"),
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
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
    }
}

fn active_camera_view_proj(resources: &Resources, aspect: f32) -> Option<Mat4> {
    let query = Query::<(&PerspectiveCamera, &GlobalTransform)>::new(resources);
    let mut best: Option<(i32, PerspectiveCamera, Mat4)> = None;
    query.for_each(|(cam, gt)| {
        if !cam.active {
            return;
        }
        let better = match &best {
            Some((p, _, _)) => cam.priority > *p,
            None => true,
        };
        if better {
            best = Some((cam.priority, *cam, gt.matrix));
        }
    });
    drop(query);

    let (_, cam, world) = best?;
    let view = world.inverse();
    let projection = kooch_render::perspective_rh_reverse_z(
        cam.fov.to_radians(),
        aspect.max(0.001),
        cam.near.max(0.001),
        cam.far.max(cam.near + 0.001),
    );
    Some(projection * view)
}

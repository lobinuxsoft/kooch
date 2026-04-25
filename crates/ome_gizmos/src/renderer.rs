use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use ome_core::resource::Resources;
use ome_ecs::PerspectiveCamera;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::query::Query;
use wgpu::util::DeviceExt;

use ome_render::VIEWPORT_DEPTH_FORMAT;

use crate::SHADER_SOURCE;

/// One line segment to be drawn in world space.
#[derive(Debug, Clone, Copy)]
pub struct LineSegment {
    pub start: Vec3,
    pub end: Vec3,
    pub color: Vec3,
}

/// Per-frame collection of line segments queued for the gizmo pass.
///
/// The editor pushes segments via [`Self::line`], [`Self::aabb`], or
/// [`Self::axis_lines`] each frame. The renderer drains the batch
/// during rendering. Stored as a `Resources` entry, inserted by the
/// editor's startup system.
#[derive(Debug, Default)]
pub struct GizmoBatch {
    pub lines: Vec<LineSegment>,
}

impl GizmoBatch {
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// Pushes a single line segment.
    pub fn line(&mut self, start: Vec3, end: Vec3, color: Vec3) {
        self.lines.push(LineSegment { start, end, color });
    }

    /// Pushes the 12 edges of an axis-aligned bounding box.
    pub fn aabb(&mut self, min: Vec3, max: Vec3, color: Vec3) {
        let corners = [
            Vec3::new(min.x, min.y, min.z),
            Vec3::new(max.x, min.y, min.z),
            Vec3::new(max.x, max.y, min.z),
            Vec3::new(min.x, max.y, min.z),
            Vec3::new(min.x, min.y, max.z),
            Vec3::new(max.x, min.y, max.z),
            Vec3::new(max.x, max.y, max.z),
            Vec3::new(min.x, max.y, max.z),
        ];
        // Bottom rect (y=min)
        self.line(corners[0], corners[1], color);
        self.line(corners[1], corners[2], color);
        self.line(corners[2], corners[3], color);
        self.line(corners[3], corners[0], color);
        // Top rect (y=max)
        self.line(corners[4], corners[5], color);
        self.line(corners[5], corners[6], color);
        self.line(corners[6], corners[7], color);
        self.line(corners[7], corners[4], color);
        // Vertical pillars
        self.line(corners[0], corners[4], color);
        self.line(corners[1], corners[5], color);
        self.line(corners[2], corners[6], color);
        self.line(corners[3], corners[7], color);
    }

    /// Pushes three world-space axis lines (X red, Y green, Z blue) of
    /// the given length starting at `origin`.
    pub fn axis_lines(&mut self, origin: Vec3, length: f32) {
        const RED: Vec3 = Vec3::new(1.0, 0.25, 0.25);
        const GREEN: Vec3 = Vec3::new(0.25, 1.0, 0.25);
        const BLUE: Vec3 = Vec3::new(0.35, 0.45, 1.0);
        self.line(origin, origin + Vec3::X * length, RED);
        self.line(origin, origin + Vec3::Y * length, GREEN);
        self.line(origin, origin + Vec3::Z * length, BLUE);
    }

    /// Pushes three world-space axis arrows (X red, Y green, Z blue) of
    /// the given length starting at `origin`. Each arrow is the main line
    /// plus four small lines forming a 3D arrowhead at the positive end.
    pub fn axis_arrows(&mut self, origin: Vec3, length: f32) {
        const RED: Vec3 = Vec3::new(1.0, 0.25, 0.25);
        const GREEN: Vec3 = Vec3::new(0.25, 1.0, 0.25);
        const BLUE: Vec3 = Vec3::new(0.35, 0.45, 1.0);
        self.arrow(origin, origin + Vec3::X * length, Vec3::Y, Vec3::Z, RED);
        self.arrow(origin, origin + Vec3::Y * length, Vec3::X, Vec3::Z, GREEN);
        self.arrow(origin, origin + Vec3::Z * length, Vec3::X, Vec3::Y, BLUE);
    }

    /// Pushes a single arrow: shaft + 4 arrowhead segments forming a
    /// "+"-shaped 3D head at `tip`. `perp_a` and `perp_b` are the two
    /// unit-length axes perpendicular to the arrow direction (orient
    /// the arrowhead).
    ///
    /// Lines render at fixed 1-pixel width — `wgpu` does not expose a
    /// line-width control. Real thickness arrives with the quad-line
    /// rendering refactor (sub-phase 3a of #278).
    pub fn arrow(&mut self, base: Vec3, tip: Vec3, perp_a: Vec3, perp_b: Vec3, color: Vec3) {
        let dir = (tip - base).normalize_or_zero();
        let length = (tip - base).length();
        let head_len = (length * 0.15).max(0.05);
        let head_w = head_len * 0.5;
        let back = tip - dir * head_len;
        self.line(base, tip, color);
        self.line(tip, back + perp_a * head_w, color);
        self.line(tip, back - perp_a * head_w, color);
        self.line(tip, back + perp_b * head_w, color);
        self.line(tip, back - perp_b * head_w, color);
    }
}

/// One vertex of a gizmo line. Position in world space, color RGB.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct GizmoVertex {
    position: [f32; 3],
    color: [f32; 3],
}

/// `view_proj` matrix matching `CameraUniforms` in `gizmo_main.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
struct CameraUniforms {
    view_proj: [[f32; 4]; 4],
}

/// Initial vertex buffer capacity in vertices (= 2 × line capacity).
/// Grows on demand if the batch overflows.
const INITIAL_VERTEX_CAPACITY: u64 = 1024;

/// Renders gizmo line segments queued in [`GizmoBatch`] each frame.
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
                topology: wgpu::PrimitiveTopology::LineList,
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

    /// Renders all queued segments. No-ops when the batch is empty or no
    /// active `PerspectiveCamera + GlobalTransform` is found.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        resources: &Resources,
        batch: &GizmoBatch,
        aspect: f32,
    ) {
        if batch.lines.is_empty() {
            return;
        }

        let Some(view_proj) = active_camera_view_proj(resources, aspect) else {
            return;
        };

        // Upload camera.
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&CameraUniforms {
                view_proj: view_proj.to_cols_array_2d(),
            }),
        );

        // Build vertex data: 2 vertices per line.
        let mut vertices: Vec<GizmoVertex> = Vec::with_capacity(batch.lines.len() * 2);
        for seg in &batch.lines {
            vertices.push(GizmoVertex {
                position: seg.start.to_array(),
                color: seg.color.to_array(),
            });
            vertices.push(GizmoVertex {
                position: seg.end.to_array(),
                color: seg.color.to_array(),
            });
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
    let projection = Mat4::perspective_rh(
        cam.fov.to_radians(),
        aspect.max(0.001),
        cam.near.max(0.001),
        cam.far.max(cam.near + 0.001),
    );
    Some(projection * view)
}

//! Mesh render pass: pipeline, bind groups, per-frame ECS query and draw loop.
//!
//! The pass appends to whatever color the previous pass wrote (`LoadOp::Load`)
//! and ignores depth — the editor's viewport stack runs the SDF pass first,
//! and meshes always paint over the SDF image until depth lands.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use ome_core::resource::Resources;
use ome_ecs::PerspectiveCamera;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::mesh_renderer::MeshRenderer;
use ome_ecs::query::Query;

use super::SHADER_SOURCE;
use super::gpu_mesh::vertex_buffer_layout;
use super::loader::MeshLoader;
use crate::VIEWPORT_DEPTH_FORMAT;

/// Camera matrices uploaded to the mesh shader. View and projection are
/// pre-multiplied to keep the shader trivial; lighting (which would need
/// view-space data) is out of scope until #130.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct MeshCameraUniforms {
    view_proj: [[f32; 4]; 4],
}

/// Slot size for the dynamic-offset model uniform buffer. Must be a
/// multiple of `min_uniform_buffer_offset_alignment` (256 on every
/// adapter we target). The actual model matrix is 64 bytes; the rest
/// is padding.
const MODEL_SLOT_SIZE: u64 = 256;
const MODEL_BIND_SIZE: u64 = 64;
const INITIAL_MODEL_CAPACITY: u32 = 4;

/// Per-frame draw record collected from the ECS before the render pass starts,
/// so we never touch the loader (mut self) while the pass borrows the pipeline.
struct DrawCall {
    model: Mat4,
    mesh_path: String,
}

/// Mesh render pass. Owns its pipeline, bind groups, camera/model uniform
/// buffers, and the [`MeshLoader`] cache.
pub struct MeshPassRenderer {
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_bg: wgpu::BindGroup,
    model_buffer: wgpu::Buffer,
    model_capacity: u32,
    model_bgl: wgpu::BindGroupLayout,
    model_bg: wgpu::BindGroup,
    loader: MeshLoader,
}

impl MeshPassRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh_camera_buffer"),
            contents: bytemuck::bytes_of(&MeshCameraUniforms::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh_camera_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(64),
                },
                count: None,
            }],
        });
        let camera_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh_camera_bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let model_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh_model_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: NonZeroU64::new(MODEL_BIND_SIZE),
                },
                count: None,
            }],
        });
        let (model_buffer, model_bg) =
            allocate_model_buffer(device, &model_bgl, INITIAL_MODEL_CAPACITY);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh_pipeline_layout"),
            bind_group_layouts: &[Some(&camera_bgl), Some(&model_bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[vertex_buffer_layout()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache,
        });

        Self {
            pipeline,
            camera_buffer,
            camera_bg,
            model_buffer,
            model_capacity: INITIAL_MODEL_CAPACITY,
            model_bgl,
            model_bg,
            loader: MeshLoader::new(),
        }
    }

    /// Records the mesh pass into `encoder`. Iterates every visible
    /// `MeshRenderer + GlobalTransform` entity, loads its mesh on demand
    /// (cached), and issues one indexed draw per entity. Skipped entirely
    /// when there is no active camera or zero visible meshes.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        resources: &Resources,
        aspect: f32,
    ) {
        let Some(view_proj) = active_camera_view_proj(resources, aspect) else {
            return;
        };

        let draws = collect_draws(resources);
        if draws.is_empty() {
            return;
        }

        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&MeshCameraUniforms {
                view_proj: view_proj.to_cols_array_2d(),
            }),
        );

        self.ensure_model_capacity(device, draws.len() as u32);

        for (i, draw) in draws.iter().enumerate() {
            let offset = i as u64 * MODEL_SLOT_SIZE;
            let mat = draw.model.to_cols_array_2d();
            queue.write_buffer(&self.model_buffer, offset, bytemuck::bytes_of(&mat));
        }

        let loaded: Vec<(u32, std::sync::Arc<super::GpuMesh>)> = draws
            .iter()
            .enumerate()
            .filter_map(|(i, d)| match self.loader.get_or_load(device, &d.mesh_path) {
                Ok(mesh) => Some((i as u32, mesh)),
                Err(e) => {
                    tracing::warn!(path = %d.mesh_path, error = %e, "skipping mesh");
                    None
                }
            })
            .collect();

        if loaded.is_empty() {
            return;
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mesh_pass"),
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
        pass.set_bind_group(0, &self.camera_bg, &[]);

        for (slot, mesh) in &loaded {
            let dyn_offset = slot * MODEL_SLOT_SIZE as u32;
            pass.set_bind_group(1, &self.model_bg, &[dyn_offset]);
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(mesh.index_buffer.slice(..), mesh.index_format);
            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
        }
    }

    fn ensure_model_capacity(&mut self, device: &wgpu::Device, needed: u32) {
        if needed <= self.model_capacity {
            return;
        }
        let new_cap = needed
            .next_power_of_two()
            .max(INITIAL_MODEL_CAPACITY);
        let (buffer, bg) = allocate_model_buffer(device, &self.model_bgl, new_cap);
        self.model_buffer = buffer;
        self.model_bg = bg;
        self.model_capacity = new_cap;
    }
}

fn allocate_model_buffer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    capacity: u32,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mesh_model_buffer"),
        size: capacity as u64 * MODEL_SLOT_SIZE,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mesh_model_bg"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: NonZeroU64::new(MODEL_BIND_SIZE),
            }),
        }],
    });
    (buffer, bg)
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

fn collect_draws(resources: &Resources) -> Vec<DrawCall> {
    let query = Query::<(&MeshRenderer, &GlobalTransform)>::new(resources);
    let mut out = Vec::new();
    query.for_each(|(mr, gt)| {
        if !mr.visible || mr.mesh.is_empty() {
            return;
        }
        out.push(DrawCall {
            model: gt.matrix,
            mesh_path: mr.mesh.clone(),
        });
    });
    out
}

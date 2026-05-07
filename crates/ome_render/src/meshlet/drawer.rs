//! Meshlet rasterizer — single-indirect-draw render pipeline that
//! consumes the cull pass's `visible_meshlets` list.
//!
//! Pairs with [`super::MeshletCull`]: the cull dispatcher writes
//! `visible_meshlets[]` and an indirect args buffer; the drawer binds
//! both and issues exactly one `draw_indirect` per frame, regardless
//! of how many meshlets were visible. Single draw call → no CPU-side
//! culling iteration → Nanite-class scaling for the rasterization
//! path.
//!
//! # Bind group layout
//!
//! ```text
//! group 0  binding 0   camera   (view_proj UBO, 64 B)
//! group 0  binding 1   model    (model UBO, 64 B)
//!
//! group 1  binding 0   vertices                (storage, read)
//! group 1  binding 1   meshlet_vertices        (storage, read)
//! group 1  binding 2   meshlet_triangles       (storage, read, u32-packed bytes)
//! group 1  binding 3   descriptors             (storage, read)
//!
//! group 2  binding 0   visible_meshlets        (storage, read)
//! ```
//!
//! Group 1 is the same layout the cull dispatcher caches via
//! [`super::MeshletCull::meshlet_bind_group_layout`] — the host-side
//! upload path is unchanged.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::dispatcher::MeshletCull;
use super::gpu_meshlet::GpuMeshletMesh;

const MESHLET_SHADER_SOURCE: &str = include_str!("../../shaders/meshlet_main.wgsl");

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct CameraUbo {
    view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct ModelUbo {
    model: [[f32; 4]; 4],
}

/// Owns the meshlet render pipeline + its UBOs + the visible-meshlets
/// bind group layout. Stateless across frames apart from the camera /
/// model UBOs, which the caller updates via [`Self::render`].
pub struct MeshletDrawer {
    pipeline: wgpu::RenderPipeline,
    camera_bgl: wgpu::BindGroupLayout,
    visible_bgl: wgpu::BindGroupLayout,
    camera_buffer: wgpu::Buffer,
    model_buffer: wgpu::Buffer,
    camera_bg: wgpu::BindGroup,
}

impl MeshletDrawer {
    /// Builds the render pipeline. `meshlet_bgl` must come from
    /// [`super::MeshletCull::meshlet_bind_group_layout`] so the cull
    /// and draw passes agree on storage-buffer slot numbering.
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: Option<wgpu::TextureFormat>,
        meshlet_bgl: &wgpu::BindGroupLayout,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet_render_shader"),
            source: wgpu::ShaderSource::Wgsl(MESHLET_SHADER_SOURCE.into()),
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_camera_ubo"),
            contents: bytemuck::bytes_of(&CameraUbo::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let model_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_model_ubo"),
            contents: bytemuck::bytes_of(&ModelUbo::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_camera_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(64),
                    },
                    count: None,
                },
            ],
        });

        let visible_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_visible_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet_render_pipeline_layout"),
            bind_group_layouts: &[Some(&camera_bgl), Some(meshlet_bgl), Some(&visible_bgl)],
            immediate_size: 0,
        });

        let depth_stencil = depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("meshlet_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_meshlet"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_meshlet"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
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
            depth_stencil,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache,
        });

        let camera_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_camera_bg"),
            layout: &camera_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: model_buffer.as_entire_binding(),
                },
            ],
        });

        Self {
            pipeline,
            camera_bgl,
            visible_bgl,
            camera_buffer,
            model_buffer,
            camera_bg,
        }
    }

    /// Bind group layout for `group(2)` — visible_meshlets storage
    /// buffer. Reusable in case future passes need to query the same
    /// list (e.g. material-pass deferred shading).
    pub fn visible_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.visible_bgl
    }

    /// Bind group layout for `group(0)` — camera + model UBO.
    pub fn camera_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.camera_bgl
    }

    /// Records one indirect-draw render pass into `encoder`. Pulls
    /// surviving meshlets from `cull` (must already have been
    /// dispatched in the same encoder before this call) and rasterizes
    /// `mesh` through the meshlet pipeline.
    ///
    /// `clear_color` controls the render-pass load op; pass `None` to
    /// keep whatever the previous pass wrote.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: Option<&wgpu::TextureView>,
        meshlet_bg: &wgpu::BindGroup,
        cull: &MeshletCull,
        view_proj: glam::Mat4,
        model: glam::Mat4,
        clear_color: Option<wgpu::Color>,
    ) {
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&CameraUbo {
                view_proj: view_proj.to_cols_array_2d(),
            }),
        );
        queue.write_buffer(
            &self.model_buffer,
            0,
            bytemuck::bytes_of(&ModelUbo {
                model: model.to_cols_array_2d(),
            }),
        );

        let visible_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_visible_bg"),
            layout: &self.visible_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: cull.visible_meshlets_buffer().as_entire_binding(),
            }],
        });

        let color_load = match clear_color {
            Some(c) => wgpu::LoadOp::Clear(c),
            None => wgpu::LoadOp::Load,
        };
        let depth_attachment = depth.map(|view| wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("meshlet_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: color_load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: depth_attachment,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bg, &[]);
        pass.set_bind_group(1, meshlet_bg, &[]);
        pass.set_bind_group(2, &visible_bg, &[]);
        pass.draw_indirect(cull.indirect_args_buffer(), 0);
    }

    /// Convenience: a `set_bind_group(1, ...)` handle built from a
    /// `GpuMeshletMesh` using the meshlet pool layout cached on the
    /// dispatcher. Useful for callers that want to render a single
    /// mesh per frame without managing bind groups themselves.
    pub fn build_meshlet_bind_group(
        &self,
        device: &wgpu::Device,
        meshlet_bgl: &wgpu::BindGroupLayout,
        mesh: &GpuMeshletMesh,
    ) -> wgpu::BindGroup {
        super::gpu_meshlet::meshlet_bind_group(device, meshlet_bgl, mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meshlet_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(MESHLET_SHADER_SOURCE)
            .expect("meshlet_main.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("meshlet_main.wgsl should validate");
    }

    #[test]
    fn camera_ubo_size_is_64_bytes() {
        // Mirror of the shader's CameraUniforms; the bind-group layout
        // declares `min_binding_size = 64`, so a drift here would fail
        // pipeline creation at runtime instead of compile time.
        assert_eq!(std::mem::size_of::<CameraUbo>(), 64);
    }

    #[test]
    fn model_ubo_size_is_64_bytes() {
        assert_eq!(std::mem::size_of::<ModelUbo>(), 64);
    }
}

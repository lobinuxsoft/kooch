//! Visibility-buffer rasterizer.
//!
//! Drop-in alternative to [`super::MeshletDrawer`]: rasterizes visible
//! meshlets to a R32Uint visibility-buffer target instead of a color
//! attachment. The fragment shader writes a packed
//! `(meshlet_id + 1) << 7 | tri_idx` value; PR-6's
//! [`super::MeshletDeferredShader`] decodes it in compute.
//!
//! # Pipeline
//!
//! ```text
//! visible_meshlets[]  →  vs_vbuf  (vertex pull, identical to forward)
//!         │
//!         ▼
//!   draw_indirect      (single call, instance_count = visible_count)
//!         │
//!         ▼
//! fs_vbuf → R32Uint visibility buffer + depth attachment
//! ```
//!
//! Standard depth test culls near-then-far meshlets; the surviving
//! pixel keeps the closest meshlet's packed id.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::dispatcher::MeshletCull;

const SHADER_SOURCE: &str = include_str!("../../shaders/meshlet_vbuf.wgsl");

/// Visibility-buffer texture format. R32Uint = one packed id per pixel.
pub const VISIBILITY_BUFFER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

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

/// Owns the vbuf rasterizer pipeline + camera/model UBOs + the
/// `visible_meshlets` bind-group layout.
pub struct MeshletVisRasterizer {
    pipeline: wgpu::RenderPipeline,
    camera_bgl: wgpu::BindGroupLayout,
    visible_bgl: wgpu::BindGroupLayout,
    camera_buffer: wgpu::Buffer,
    model_buffer: wgpu::Buffer,
    camera_bg: wgpu::BindGroup,
}

impl MeshletVisRasterizer {
    /// `meshlet_bgl` must come from
    /// [`MeshletCull::meshlet_bind_group_layout`] so the vbuf and
    /// forward paths share the storage-buffer slot numbering.
    pub fn new(
        device: &wgpu::Device,
        depth_format: Option<wgpu::TextureFormat>,
        meshlet_bgl: &wgpu::BindGroupLayout,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet_vbuf_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_vbuf_camera_ubo"),
            contents: bytemuck::bytes_of(&CameraUbo::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let model_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_vbuf_model_ubo"),
            contents: bytemuck::bytes_of(&ModelUbo::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_vbuf_camera_bgl"),
            entries: &[
                ubo_entry(0, 64),
                ubo_entry(1, 64),
            ],
        });
        let visible_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_vbuf_visible_bgl"),
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
            label: Some("meshlet_vbuf_pipeline_layout"),
            bind_group_layouts: &[Some(&camera_bgl), Some(meshlet_bgl), Some(&visible_bgl)],
            immediate_size: 0,
        });

        let depth_stencil = depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("meshlet_vbuf_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_vbuf"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_vbuf"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: VISIBILITY_BUFFER_FORMAT,
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
            label: Some("meshlet_vbuf_camera_bg"),
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

    pub fn camera_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.camera_bgl
    }

    pub fn visible_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.visible_bgl
    }

    /// Records one indirect-draw render pass that writes the
    /// visibility buffer. `vbuf_view` must reference an R32Uint texture
    /// matching [`VISIBILITY_BUFFER_FORMAT`]. `clear_id` is the value
    /// the pass clears the buffer to (use `0` for "background").
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        vbuf_view: &wgpu::TextureView,
        depth: Option<&wgpu::TextureView>,
        meshlet_bg: &wgpu::BindGroup,
        cull: &MeshletCull,
        view_proj: glam::Mat4,
        model: glam::Mat4,
        clear_id: u32,
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
            label: Some("meshlet_vbuf_visible_bg"),
            layout: &self.visible_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: cull.visible_meshlets_buffer().as_entire_binding(),
            }],
        });

        let depth_attachment = depth.map(|view| wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("meshlet_vbuf_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: vbuf_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: clear_id as f64,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }),
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
}

fn ubo_entry(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(size),
        },
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vbuf_shader_parses_and_validates() {
        let module =
            naga::front::wgsl::parse_str(SHADER_SOURCE).expect("meshlet_vbuf.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("meshlet_vbuf.wgsl should validate");
    }

    #[test]
    fn vbuf_format_is_r32uint() {
        assert_eq!(VISIBILITY_BUFFER_FORMAT, wgpu::TextureFormat::R32Uint);
    }
}

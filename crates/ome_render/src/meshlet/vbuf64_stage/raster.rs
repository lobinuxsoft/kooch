//! Atomic R64 visibility-buffer rasterizer (#493).
//!
//! Mirrors Bevy's meshlet pipeline: the fragment writes a packed u64 to
//! the storage R64 vbuf via `textureAtomicMax`, so the closest fragment
//! per pixel wins atomically under reversed-Z. No color attachment; the
//! depth attachment is kept so hardware early-Z still elides occluded
//! fragments before they reach the atomic.
//!
//! Bind groups (matches `meshlet_vbuf64.wgsl`):
//!   group(0) — camera UBO
//!   group(1) — meshlet pool (shared with cull / R32 raster)
//!   group(2) — visible_meshlets storage (from cull)
//!   group(3) — instances storage (from scene)
//!   group(4) — vbuf64 storage texture, atomic access

use std::num::NonZeroU64;

use bytemuck::bytes_of;
use wgpu::util::DeviceExt;

use crate::meshlet::dispatcher::MeshletCull;
use crate::meshlet::scene::MeshletScene;

use super::CameraUbo;

const SHADER_SOURCE: &str = include_str!("../../../shaders/meshlet_vbuf64.wgsl");

pub(super) struct Vbuf64Rasterizer {
    pipeline: wgpu::RenderPipeline,
    visible_bgl: wgpu::BindGroupLayout,
    instances_bgl: wgpu::BindGroupLayout,
    vbuf_bgl: wgpu::BindGroupLayout,
    density_bgl: wgpu::BindGroupLayout,
    camera_buffer: wgpu::Buffer,
    camera_bg: wgpu::BindGroup,
    /// 16-byte UBO storing the per-frame density-enable flag (only
    /// `.x` is read by the shader). Written from `render_scene` so the
    /// production path leaves the atomicAdd dormant.
    density_enable_buffer: wgpu::Buffer,
}

impl Vbuf64Rasterizer {
    pub(super) fn new(
        device: &wgpu::Device,
        meshlet_bgl: &wgpu::BindGroupLayout,
        depth_format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet_vbuf64_raster_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_vbuf64_camera_ubo"),
            contents: bytes_of(&CameraUbo::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_vbuf64_camera_bgl"),
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
        let visible_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_vbuf64_visible_bgl"),
            entries: &[storage_entry_vertex(0)],
        });
        let instances_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_vbuf64_instances_bgl"),
            entries: &[storage_entry_vertex(0)],
        });
        let vbuf_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_vbuf64_target_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::Atomic,
                    format: super::VBUF64_FORMAT,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            }],
        });
        // #454 — Bind group 5: triangle-density accumulator + the
        // uniform that gates the atomicAdd. Both are bound on every
        // frame regardless of the active debug mode; the uniform
        // disables the accumulation for production rendering so the
        // hot path costs at most one uniform fetch and a predicted
        // branch.
        let density_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_vbuf64_density_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::Atomic,
                        format: wgpu::TextureFormat::R32Uint,
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
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet_vbuf64_pipeline_layout"),
            bind_group_layouts: &[
                Some(&camera_bgl),
                Some(meshlet_bgl),
                Some(&visible_bgl),
                Some(&instances_bgl),
                Some(&vbuf_bgl),
                Some(&density_bgl),
            ],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("meshlet_vbuf64_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_vbuf64_scene"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_vbuf64_scene"),
                // Bevy convention (#493): wgpu / Vulkan reject a render
                // pipeline with zero color targets when there is a
                // fragment stage, even though our fragment writes via
                // textureAtomicMax through a storage binding instead of
                // a return value. Declaring a dummy R8Uint target with
                // an empty write_mask satisfies the validation; the
                // attachment is bound at render-pass time but every
                // write is masked off so it stays cleared / undefined.
                targets: &[Some(wgpu::ColorTargetState {
                    format: super::DUMMY_COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                // Reversed-Z: closer fragments have larger NDC z, so
                // pass when the new fragment is greater than what the
                // depth buffer holds. Matches the R32 path.
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache,
        });

        let camera_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_vbuf64_camera_bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let density_enable_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_vbuf64_density_enable_ubo"),
            contents: bytes_of(&[0u32; 4]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            pipeline,
            visible_bgl,
            instances_bgl,
            vbuf_bgl,
            density_bgl,
            camera_buffer,
            camera_bg,
            density_enable_buffer,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_scene(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        vbuf64_view: &wgpu::TextureView,
        dummy_color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        density_view: &wgpu::TextureView,
        density_enable: bool,
        meshlet_bg: &wgpu::BindGroup,
        cull: &MeshletCull,
        scene: &MeshletScene,
        view_proj: glam::Mat4,
        clear_depth: bool,
    ) {
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytes_of(&CameraUbo {
                view_proj: view_proj.to_cols_array_2d(),
            }),
        );
        // `vec4<u32>` so the shader-side layout matches a 16-byte UBO
        // (uniform std140 minimum). Only `.x` is read.
        let density_flag = [u32::from(density_enable), 0u32, 0u32, 0u32];
        queue.write_buffer(&self.density_enable_buffer, 0, bytes_of(&density_flag));

        let visible_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_vbuf64_visible_bg"),
            layout: &self.visible_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: cull.visible_meshlets_buffer().as_entire_binding(),
            }],
        });
        let instances_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_vbuf64_instances_bg"),
            layout: &self.instances_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene.instance_buffer().as_entire_binding(),
            }],
        });
        let vbuf_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_vbuf64_target_bg"),
            layout: &self.vbuf_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(vbuf64_view),
            }],
        });
        let density_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_vbuf64_density_bg"),
            layout: &self.density_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(density_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.density_enable_buffer.as_entire_binding(),
                },
            ],
        });

        let depth_load = if clear_depth {
            wgpu::LoadOp::Clear(0.0)
        } else {
            wgpu::LoadOp::Load
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("meshlet_vbuf64_pass"),
            // Dummy color attachment to match the pipeline's declared
            // target. Every write is masked off so the load/store ops
            // are immaterial — `LoadOp::Clear(0)` keeps the texture in
            // a defined state if anything else samples it.
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dummy_color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Discard,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: depth_load,
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
        pass.set_bind_group(1, meshlet_bg, &[]);
        pass.set_bind_group(2, &visible_bg, &[]);
        pass.set_bind_group(3, &instances_bg, &[]);
        pass.set_bind_group(4, &vbuf_bg, &[]);
        pass.set_bind_group(5, &density_bg, &[]);
        pass.draw_indirect(cull.indirect_args_buffer(), 0);
    }
}

fn storage_entry_vertex(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

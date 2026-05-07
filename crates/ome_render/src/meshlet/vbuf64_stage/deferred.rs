//! Compute shading pass that reads from the atomic R64 vbuf (#493).
//!
//! Mirrors `MeshletDeferredShader::shade_scene` but binds the vbuf as a
//! `texture_storage_2d<r64uint, atomic>` (read access via `textureLoad`)
//! instead of the legacy R32Uint sampled texture. The `(visible_slot,
//! tri_idx)` decoding is identical save for the missing +1 sentinel
//! offset — the R64 path uses `packed == 0` as the background sentinel
//! since cleared vbuf + reversed-Z far plane both produce 0.

use bytemuck::bytes_of;

use crate::material::MaterialPool;
use crate::meshlet::dispatcher::MeshletCull;
use crate::meshlet::scene::MeshletScene;

use super::{CameraUbo, ScreenUbo};

const SHADER_SOURCE: &str = include_str!("../../../shaders/meshlet_deferred_r64.wgsl");

pub(super) struct Vbuf64Deferred {
    pipeline: wgpu::ComputePipeline,
    shading_bgl: wgpu::BindGroupLayout,
    scene_bgl: wgpu::BindGroupLayout,
    camera_buffer: wgpu::Buffer,
    screen_buffer: wgpu::Buffer,
}

impl Vbuf64Deferred {
    pub(super) fn new(
        device: &wgpu::Device,
        meshlet_bgl: &wgpu::BindGroupLayout,
    ) -> Self {
        use wgpu::util::DeviceExt;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet_deferred_r64_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_deferred_r64_camera_ubo"),
            contents: bytes_of(&CameraUbo::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let screen_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_deferred_r64_screen_ubo"),
            contents: bytes_of(&ScreenUbo::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let shading_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_deferred_r64_shading_bgl"),
            entries: &[
                ubo_entry(0, 64),
                ubo_entry(1, 16),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::Atomic,
                        format: super::VBUF64_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: super::DEFERRED_COLOR_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        let material_bgl = MaterialPool::bind_group_layout(device);

        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_deferred_r64_scene_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet_deferred_r64_pipeline_layout"),
            bind_group_layouts: &[
                Some(&shading_bgl),
                Some(meshlet_bgl),
                Some(&material_bgl),
                Some(&scene_bgl),
            ],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("meshlet_deferred_r64_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_shade_scene_r64"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            pipeline,
            shading_bgl,
            scene_bgl,
            camera_buffer,
            screen_buffer,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn shade_scene(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        vbuf64_view: &wgpu::TextureView,
        color_view: &wgpu::TextureView,
        meshlet_bg: &wgpu::BindGroup,
        material_bg: &wgpu::BindGroup,
        cull: &MeshletCull,
        scene: &MeshletScene,
        view_proj: glam::Mat4,
        screen_size: (u32, u32),
        debug_mode: u32,
    ) {
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytes_of(&CameraUbo {
                view_proj: view_proj.to_cols_array_2d(),
            }),
        );
        queue.write_buffer(
            &self.screen_buffer,
            0,
            bytes_of(&ScreenUbo {
                size: [screen_size.0, screen_size.1],
                material_id: 0,
                debug_mode,
            }),
        );

        let shading_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_deferred_r64_shading_bg"),
            layout: &self.shading_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.screen_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(vbuf64_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(color_view),
                },
            ],
        });
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_deferred_r64_scene_bg"),
            layout: &self.scene_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cull.visible_meshlets_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: scene.instance_buffer().as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("meshlet_deferred_r64_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &shading_bg, &[]);
        pass.set_bind_group(1, meshlet_bg, &[]);
        pass.set_bind_group(2, material_bg, &[]);
        pass.set_bind_group(3, &scene_bg, &[]);
        pass.dispatch_workgroups(screen_size.0.div_ceil(8), screen_size.1.div_ceil(8), 1);
    }
}

fn ubo_entry(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: std::num::NonZeroU64::new(size),
        },
        count: None,
    }
}

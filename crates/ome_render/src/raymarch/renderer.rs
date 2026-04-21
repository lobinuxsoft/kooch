//! Ray-march render pipeline + buffers + bind groups.

use wgpu::util::DeviceExt;

use super::SHADER_SOURCE;
use super::instance::{
    CameraUniforms, INITIAL_INSTANCE_CAPACITY, RayMarchParams, SceneMeta, SdfInstance,
};

/// Ray-marching pipeline + buffers + bind groups.
pub struct RayMarchRenderer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) camera_buffer: wgpu::Buffer,
    pub(super) params_buffer: wgpu::Buffer,
    pub(super) scene_meta_buffer: wgpu::Buffer,
    pub(super) instance_buffer: wgpu::Buffer,
    pub(super) instance_capacity: u64,
    pub(super) scene_bind_group_layout: wgpu::BindGroupLayout,
    pub(super) camera_bind_group: wgpu::BindGroup,
    pub(super) scene_bind_group: wgpu::BindGroup,
    pub params: RayMarchParams,
}

impl RayMarchRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raymarch_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("raymarch_camera_buffer"),
            contents: bytemuck::bytes_of(&CameraUniforms::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("raymarch_params_buffer"),
            contents: bytemuck::bytes_of(&RayMarchParams::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let scene_meta_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("raymarch_scene_meta_buffer"),
            contents: bytemuck::bytes_of(&SceneMeta::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("raymarch_instance_buffer"),
            size: INITIAL_INSTANCE_CAPACITY * std::mem::size_of::<SdfInstance>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("raymarch_camera_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let scene_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("raymarch_scene_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
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

        let camera_bind_group = make_camera_bg(
            device,
            &camera_bind_group_layout,
            &camera_buffer,
            &params_buffer,
        );
        let scene_bind_group = make_scene_bg(
            device,
            &scene_bind_group_layout,
            &scene_meta_buffer,
            &instance_buffer,
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raymarch_pipeline_layout"),
            bind_group_layouts: &[&camera_bind_group_layout, &scene_bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("raymarch_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
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
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            camera_buffer,
            params_buffer,
            scene_meta_buffer,
            instance_buffer,
            instance_capacity: INITIAL_INSTANCE_CAPACITY,
            scene_bind_group_layout,
            camera_bind_group,
            scene_bind_group,
            params: RayMarchParams::default(),
        }
    }

    /// Records the ray-march pass into `encoder`, clearing the target to
    /// black and drawing the fullscreen triangle.
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("raymarch_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_bind_group(1, &self.scene_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

pub(super) fn make_camera_bg(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    camera: &wgpu::Buffer,
    params: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("raymarch_camera_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: camera.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params.as_entire_binding(),
            },
        ],
    })
}

pub(super) fn make_scene_bg(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    meta: &wgpu::Buffer,
    instances: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("raymarch_scene_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: meta.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: instances.as_entire_binding(),
            },
        ],
    })
}

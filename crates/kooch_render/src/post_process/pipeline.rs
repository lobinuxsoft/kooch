//! The pipeline a post-process shader is built into, and the bindings it reads (#1201).
//!
//! 🔴 Groups 1 and 3 are empty and still named: the surface contract puts the material storage at 2
//! and the material's textures at 4, so a post-process keeps its parameters and its textures by
//! landing on the same numbers the scene's shading uses.

use crate::material::{MaterialPipeline, MaterialTexturePool};
use crate::meshlet::compose_post_shader;

/// What the frame tells the shader about itself.
#[derive(Clone, Copy)]
pub struct PostUniforms {
    pub resolution: [f32; 2],
    pub time: f32,
    pub material_id: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ScreenUbo {
    material_id: u32,
    mip_bias_scale: f32,
    time: f32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct IntiUbo {
    camera_position: [f32; 3],
    _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ResolutionUbo {
    resolution: [f32; 2],
    _pad: [f32; 2],
}

/// One effect's uniforms. Per effect, not shared: a stack records several passes into one encoder,
/// and `write_buffer` lands before the submit, so a shared buffer would give every pass the last
/// effect's values.
pub(super) struct Uniforms {
    screen: wgpu::Buffer,
    inti: wgpu::Buffer,
    resolution: wgpu::Buffer,
}

/// Layouts and samplers: everything a post-process pipeline needs that does not depend on which
/// shader is showing.
pub(super) struct Parts {
    frame_bgl: wgpu::BindGroupLayout,
    materials_bgl: wgpu::BindGroupLayout,
    texture_bgl: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    empty_bg: wgpu::BindGroup,
}

impl Parts {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let frame_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post_process_frame_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform(2),
                uniform(3),
                uniform(4),
            ],
        });
        let empty_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post_process_empty_bgl"),
            entries: &[],
        });
        let materials_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post_process_materials_bgl"),
            entries: &[storage_read(0), storage_read(1)],
        });
        let texture_bgl = MaterialTexturePool::bind_group_layout(device);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post_process_layout"),
            bind_group_layouts: &[
                Some(&frame_bgl),
                Some(&empty_bgl),
                Some(&materials_bgl),
                Some(&empty_bgl),
                Some(&texture_bgl),
            ],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post_process_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let empty_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post_process_empty_bg"),
            layout: &empty_bgl,
            entries: &[],
        });
        Self {
            frame_bgl,
            materials_bgl,
            texture_bgl,
            layout,
            sampler,
            empty_bg,
        }
    }

    pub(super) fn uniforms(&self, device: &wgpu::Device) -> Uniforms {
        let buffer = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Uniforms {
            screen: buffer("post_process_screen"),
            inti: buffer("post_process_inti"),
            resolution: buffer("post_process_resolution"),
        }
    }

    pub(super) fn write_uniforms(
        &self,
        queue: &wgpu::Queue,
        uniforms: &Uniforms,
        values: PostUniforms,
    ) {
        queue.write_buffer(
            &uniforms.screen,
            0,
            bytemuck::bytes_of(&ScreenUbo {
                material_id: values.material_id,
                // A post-process samples at screen size, so there is no bias to apply.
                mip_bias_scale: 1.0,
                time: values.time,
                _pad: 0,
            }),
        );
        queue.write_buffer(
            &uniforms.inti,
            0,
            bytemuck::bytes_of(&IntiUbo {
                camera_position: [0.0; 3],
                _pad: 0.0,
            }),
        );
        queue.write_buffer(
            &uniforms.resolution,
            0,
            bytemuck::bytes_of(&ResolutionUbo {
                resolution: values.resolution,
                _pad: [0.0; 2],
            }),
        );
    }

    /// The five groups, in the order the pass sets them.
    pub(super) fn bind_groups(
        &self,
        device: &wgpu::Device,
        uniforms: &Uniforms,
        scene: &wgpu::TextureView,
        materials: &MaterialPipeline,
        slot: u32,
    ) -> [wgpu::BindGroup; 5] {
        let frame = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post_process_frame_bg"),
            layout: &self.frame_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(scene),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                uniforms.screen.as_entire_binding().into_entry(2),
                uniforms.inti.as_entire_binding().into_entry(3),
                uniforms.resolution.as_entire_binding().into_entry(4),
            ],
        });
        let materials_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post_process_materials_bg"),
            layout: &self.materials_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: materials.pool().buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: materials.pool().values().as_entire_binding(),
                },
            ],
        });
        // The material's own textures, so a Texture node reads what the Inspector assigned.
        let textures = materials
            .texture_pool()
            .material_bind_group(device, &materials.slot_texture_refs(slot));
        [
            frame,
            self.empty_bg.clone(),
            materials_bg,
            self.empty_bg.clone(),
            textures,
        ]
    }

    pub(super) fn build(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        params_wgsl: &str,
        source: &str,
    ) -> wgpu::RenderPipeline {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post_process"),
            source: wgpu::ShaderSource::Wgsl(compose_post_shader(params_wgsl, source).into()),
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("post_process"),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_fullscreen"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_post"),
                targets: &[Some(format.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        })
    }
}

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_read(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// `BindingResource` to `BindGroupEntry`, so the list above reads as a list.
trait IntoEntry<'a> {
    fn into_entry(self, binding: u32) -> wgpu::BindGroupEntry<'a>;
}

impl<'a> IntoEntry<'a> for wgpu::BindingResource<'a> {
    fn into_entry(self, binding: u32) -> wgpu::BindGroupEntry<'a> {
        wgpu::BindGroupEntry {
            binding,
            resource: self,
        }
    }
}

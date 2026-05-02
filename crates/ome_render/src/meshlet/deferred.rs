//! Visibility-buffer compute shading pass.
//!
//! Reads the packed `(meshlet_id, triangle_id)` from a R32Uint
//! visibility buffer (output by [`super::MeshletVisRasterizer`]), looks
//! up the triangle's three vertex normals in the meshlet pool,
//! averages them (PR-6 minimal — bary-correct interpolation in PR-7),
//! and writes a normal-debug RGBA8 color.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

const SHADER_SOURCE: &str = include_str!("../../shaders/meshlet_deferred.wgsl");

/// Output color format the deferred shader writes through a storage
/// texture binding. Rgba8Unorm matches the forward path so tests can
/// compare pixel-for-pixel.
pub const DEFERRED_COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

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

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct ScreenUbo {
    size: [u32; 2],
    material_id: u32,
    _pad: u32,
}

/// Owns the deferred-shading compute pipeline and its UBOs.
pub struct MeshletDeferredShader {
    pipeline: wgpu::ComputePipeline,
    shading_bgl: wgpu::BindGroupLayout,

    camera_buffer: wgpu::Buffer,
    model_buffer: wgpu::Buffer,
    screen_buffer: wgpu::Buffer,
}

impl MeshletDeferredShader {
    /// `meshlet_bgl` must come from
    /// [`super::MeshletCull::meshlet_bind_group_layout`] so the meshlet
    /// pool layout is shared with the rasterizer.
    pub fn new(device: &wgpu::Device, meshlet_bgl: &wgpu::BindGroupLayout) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet_deferred_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_deferred_camera_ubo"),
            contents: bytemuck::bytes_of(&CameraUbo::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let model_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_deferred_model_ubo"),
            contents: bytemuck::bytes_of(&ModelUbo::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let screen_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_deferred_screen_ubo"),
            contents: bytemuck::bytes_of(&ScreenUbo::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let shading_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_deferred_shading_bgl"),
            entries: &[
                ubo_entry(0, 64),
                ubo_entry(1, 64),
                ubo_entry(2, 16),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: DEFERRED_COLOR_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        let material_bgl = crate::material::MaterialPool::bind_group_layout(device);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet_deferred_pipeline_layout"),
            bind_group_layouts: &[Some(&shading_bgl), Some(meshlet_bgl), Some(&material_bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("meshlet_deferred_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_shade"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            pipeline,
            shading_bgl,
            camera_buffer,
            model_buffer,
            screen_buffer,
        }
    }

    /// Records the compute shading pass into `encoder`. `vbuf_view`
    /// reads the visibility buffer; `color_view` is the
    /// storage-texture write target. `material_bg` is built from a
    /// [`crate::material::MaterialPool`]; `material_id` selects which
    /// pool slot drives this render call.
    #[allow(clippy::too_many_arguments)]
    pub fn shade(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        vbuf_view: &wgpu::TextureView,
        color_view: &wgpu::TextureView,
        meshlet_bg: &wgpu::BindGroup,
        material_bg: &wgpu::BindGroup,
        view_proj: glam::Mat4,
        model: glam::Mat4,
        screen_size: (u32, u32),
        material_id: u32,
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
        queue.write_buffer(
            &self.screen_buffer,
            0,
            bytemuck::bytes_of(&ScreenUbo {
                size: [screen_size.0, screen_size.1],
                material_id,
                _pad: 0,
            }),
        );

        let shading_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_deferred_shading_bg"),
            layout: &self.shading_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.model_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.screen_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(vbuf_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(color_view),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("meshlet_deferred_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &shading_bg, &[]);
        pass.set_bind_group(1, meshlet_bg, &[]);
        pass.set_bind_group(2, material_bg, &[]);
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
            min_binding_size: NonZeroU64::new(size),
        },
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(SHADER_SOURCE)
            .expect("meshlet_deferred.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("meshlet_deferred.wgsl should validate");
    }

    #[test]
    fn screen_ubo_layout() {
        assert_eq!(std::mem::size_of::<ScreenUbo>(), 16);
    }
}

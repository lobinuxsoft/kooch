//! Visibility-buffer compute shading pass.
//!
//! Reads the packed value from a R32Uint visibility buffer (output by
//! [`super::MeshletVisRasterizer`]), looks up the triangle's three
//! vertex normals in the meshlet pool, averages them, and writes a
//! normal-debug RGBA8 color modulated by the material's base colour.
//!
//! # Two paths share the shader
//!
//! - **Single-mesh** (`shade`, shader entry `cs_shade`): packed pixel
//!   carries `meshlet_id+1`. Per-render-call `model` matrix +
//!   `material_id` come from UBOs.
//! - **Scene-wide** (`shade_scene`, shader entry `cs_shade_scene`):
//!   packed pixel carries `visible_slot+1`. The shader resolves
//!   `(instance_id, meshlet_idx)` via `visible_meshlets[]` and reads
//!   per-instance transform / material_id from `instances[]`.

mod scene;

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

/// This path's own half of the shader: the R32Uint visibility read and
/// the two compute entry points.
const DEFERRED_BODY: &str = include_str!("../../../shaders/meshlet_deferred.wgsl");

/// Bind group Inti occupies here. Groups 0..3 are the shading bindings,
/// the meshlet pool, the material storage and the scene buffers — 4 is
/// the first free index. Differs from the two-pass path's group 5
/// because that path also binds per-material textures; the WGSL is the
/// same text with a different number substituted in.
pub const DEFERRED_INTI_GROUP: u32 = 4;

/// Group-0 bindings the contact-shadow march takes on this path (#735).
/// 0..4 are the camera, model and screen uniforms, the visibility buffer
/// and the colour target; these are the next free. Group 0 because the
/// depth buffer is per view, exactly as on the R64 path — the two paths
/// differ in the *numbers*, and in nothing else.
pub const DEFERRED_CONTACT_UBO_BINDING: u32 = 5;
pub const DEFERRED_CONTACT_DEPTH_BINDING: u32 = 6;

/// The complete compute shader: shared barycentric reconstruction, the
/// Inti shading model, then this path's entry points.
///
/// The reconstruction chunk is the same text the R64 fragment path
/// composes. Before #441 this path averaged the triangle's three vertex
/// normals and never computed a world position — fine for shading that
/// only read the normal, wrong the moment a point light needs a
/// distance. Sharing the chunk is what stops the fallback from quietly
/// drifting away from the path everyone actually looks at.
fn shader_source() -> String {
    [
        crate::meshlet::SURFACE_RECONSTRUCT_SHADER,
        &crate::contact_shadow::contact_shadow_shader(
            DEFERRED_CONTACT_UBO_BINDING,
            DEFERRED_CONTACT_DEPTH_BINDING,
        ),
        &kooch_lighting::inti_pbr_shader(DEFERRED_INTI_GROUP),
        DEFERRED_BODY,
    ]
    .join("\n")
}

/// Output color format the deferred shader writes through a storage
/// texture binding. Rgba8Unorm matches the forward path so tests can
/// compare pixel-for-pixel.
pub const DEFERRED_COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub(super) struct CameraUbo {
    pub view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub(super) struct ModelUbo {
    pub model: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub(super) struct ScreenUbo {
    pub size: [u32; 2],
    pub material_id: u32,
    /// Debug-visualization mode discriminant (#451). 0 = Off.
    pub debug_mode: u32,
}

/// Owns the deferred-shading compute pipelines (single-mesh + scene)
/// and their UBOs.
pub struct MeshletDeferredShader {
    pub(super) pipeline: wgpu::ComputePipeline,
    pub(super) pipeline_scene: wgpu::ComputePipeline,
    pub(super) shading_bgl: wgpu::BindGroupLayout,
    pub(super) scene_bgl: wgpu::BindGroupLayout,

    pub(super) camera_buffer: wgpu::Buffer,
    pub(super) model_buffer: wgpu::Buffer,
    pub(super) screen_buffer: wgpu::Buffer,
    /// The contact-shadow march's per-view uniform (#735).
    pub(super) contact_buffer: wgpu::Buffer,
}

impl MeshletDeferredShader {
    /// `meshlet_bgl` must come from
    /// [`super::MeshletCull::meshlet_bind_group_layout`] so the meshlet
    /// pool layout is shared with the rasterizer.
    pub fn new(device: &wgpu::Device, meshlet_bgl: &wgpu::BindGroupLayout) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet_deferred_shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
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

        let contact_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_deferred_contact_shadow_ubo"),
            contents: bytemuck::bytes_of(&crate::contact_shadow::ContactShadowUbo::default()),
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
                ubo_entry(
                    DEFERRED_CONTACT_UBO_BINDING,
                    std::mem::size_of::<crate::contact_shadow::ContactShadowUbo>() as u64,
                ),
                wgpu::BindGroupLayoutEntry {
                    binding: DEFERRED_CONTACT_DEPTH_BINDING,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let material_bgl = crate::material::MaterialPool::bind_group_layout(device);

        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_deferred_scene_bgl"),
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
            label: Some("meshlet_deferred_pipeline_layout"),
            bind_group_layouts: &[Some(&shading_bgl), Some(meshlet_bgl), Some(&material_bgl)],
            immediate_size: 0,
        });
        let lights_bgl = kooch_lighting::GpuLights::bind_group_layout(device);
        let pipeline_layout_scene =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meshlet_deferred_pipeline_layout_scene"),
                bind_group_layouts: &[
                    Some(&shading_bgl),
                    Some(meshlet_bgl),
                    Some(&material_bgl),
                    Some(&scene_bgl),
                    Some(&lights_bgl),
                ],
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
        let pipeline_scene = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("meshlet_deferred_pipeline_scene"),
            layout: Some(&pipeline_layout_scene),
            module: &shader,
            entry_point: Some("cs_shade_scene"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            pipeline,
            pipeline_scene,
            shading_bgl,
            scene_bgl,
            camera_buffer,
            model_buffer,
            screen_buffer,
            contact_buffer,
        }
    }

    /// Bind-group layout for the scene path's `group(3)` —
    /// `visible_meshlets` storage at binding 0 + `instances` storage
    /// at binding 1.
    pub fn scene_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.scene_bgl
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
        depth_sample_view: &wgpu::TextureView,
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
                // Single-mesh path is always production-shaded; debug
                // modes only ship through the scene path.
                debug_mode: 0,
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
                wgpu::BindGroupEntry {
                    binding: DEFERRED_CONTACT_UBO_BINDING,
                    resource: self.contact_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: DEFERRED_CONTACT_DEPTH_BINDING,
                    resource: wgpu::BindingResource::TextureView(depth_sample_view),
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
        let module = naga::front::wgsl::parse_str(&shader_source())
            .expect("composed meshlet_deferred shader should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("composed meshlet_deferred shader should validate");
    }

    #[test]
    fn screen_ubo_layout() {
        assert_eq!(std::mem::size_of::<ScreenUbo>(), 16);
    }
}

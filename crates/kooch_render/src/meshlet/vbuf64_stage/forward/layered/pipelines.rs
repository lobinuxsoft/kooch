//! The layered passes' layouts and pipelines (#452).

use crate::material::SurfaceSource;
use crate::meshlet::deferred::HDR_COLOR_FORMAT;
use crate::meshlet::{
    MATERIAL_PASS_CONTACT_DEPTH_BINDING, MATERIAL_PASS_CONTACT_UBO_BINDING,
    TRANSPARENT_ARGS_SHADER, TRANSPARENT_SHADE_FRAME, TRANSPARENT_TAIL_FRAME,
    compose_material_shader, compose_transparent_composite, compose_transparent_insert,
};

use super::super::super::{ScreenUbo, VBUF64_FORMAT};

/// The tail's accumulated colour and weight.
pub(super) const ACCUM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The light the tail lets through.
pub(super) const REVEAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// Group-0 bindings past the ones the material frames already use.
pub(super) const LAYERS_BINDING: u32 = 5;
pub(super) const OVERFLOW_BINDING: u32 = 6;

const SHADING: wgpu::ShaderStages = wgpu::ShaderStages::FRAGMENT.union(wgpu::ShaderStages::COMPUTE);
const EVERY: wgpu::ShaderStages = SHADING.union(wgpu::ShaderStages::VERTEX);

fn buffer(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

const READ: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: true };
const WRITE: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: false };

/// The layouts every layered pass binds.
pub(super) struct Layouts {
    pub frame: wgpu::BindGroupLayout,
    pub materials: wgpu::BindGroupLayout,
    pub scene: wgpu::BindGroupLayout,
    pub composite: wgpu::BindGroupLayout,
    pub args: wgpu::BindGroupLayout,
    /// Frame, pool, materials, scene, textures, lights: insert, tail and shade.
    pub material: wgpu::PipelineLayout,
}

impl Layouts {
    pub(super) fn new(device: &wgpu::Device, meshlet_bgl: &wgpu::BindGroupLayout) -> Self {
        let frame = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transparent_frame_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: SHADING,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::ReadOnly,
                        format: VBUF64_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                buffer(1, EVERY, wgpu::BufferBindingType::Uniform),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: EVERY,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<ScreenUbo>() as u64,
                        ),
                    },
                    count: None,
                },
                buffer(
                    MATERIAL_PASS_CONTACT_UBO_BINDING,
                    SHADING,
                    wgpu::BufferBindingType::Uniform,
                ),
                wgpu::BindGroupLayoutEntry {
                    binding: MATERIAL_PASS_CONTACT_DEPTH_BINDING,
                    visibility: SHADING,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                buffer(LAYERS_BINDING, SHADING, WRITE),
                buffer(OVERFLOW_BINDING, SHADING, WRITE),
            ],
        });
        let materials = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transparent_materials_bgl"),
            entries: &[buffer(0, SHADING, READ), buffer(1, SHADING, READ)],
        });
        let scene = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transparent_scene_bgl"),
            entries: &[buffer(0, EVERY, READ), buffer(1, EVERY, READ)],
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let composite = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transparent_composite_bgl"),
            entries: &[
                buffer(0, wgpu::ShaderStages::FRAGMENT, READ),
                texture(1),
                texture(2),
                buffer(
                    3,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
            ],
        });
        let args = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transparent_args_bgl"),
            entries: &[
                buffer(0, wgpu::ShaderStages::COMPUTE, READ),
                buffer(1, wgpu::ShaderStages::COMPUTE, WRITE),
            ],
        });
        let textures = crate::material::MaterialTexturePool::bind_group_layout(device);
        let lights = kooch_lighting::GpuLights::bind_group_layout(device);
        let material = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("transparent_material_layout"),
            bind_group_layouts: &[
                Some(&frame),
                Some(meshlet_bgl),
                Some(&materials),
                Some(&scene),
                Some(&textures),
                Some(&lights),
            ],
            immediate_size: 0,
        });
        Self {
            frame,
            materials,
            scene,
            composite,
            args,
            material,
        }
    }
}

/// Transparent geometry is tested against the opaque depth and never writes it.
fn depth_read(format: wgpu::TextureFormat) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format,
        depth_write_enabled: Some(false),
        // Reversed-Z, as the raster that wrote it.
        depth_compare: Some(wgpu::CompareFunction::Greater),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

/// Both faces: the layers put them in order per pixel, so nothing is culled for the sake of it.
fn both_faces() -> wgpu::PrimitiveState {
    wgpu::PrimitiveState {
        topology: wgpu::PrimitiveTopology::TriangleList,
        cull_mode: None,
        ..Default::default()
    }
}

fn module(device: &wgpu::Device, label: &str, source: String) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

pub(super) fn insert(
    device: &wgpu::Device,
    layouts: &Layouts,
    depth_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let module = module(device, "transparent_insert", compose_transparent_insert());
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("transparent_insert"),
        layout: Some(&layouts.material),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_forward"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_insert"),
            targets: &[],
            compilation_options: Default::default(),
        }),
        primitive: both_faces(),
        depth_stencil: Some(depth_read(depth_format)),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

pub(super) fn tail(
    device: &wgpu::Device,
    layouts: &Layouts,
    depth_format: wgpu::TextureFormat,
    surface: &SurfaceSource,
) -> wgpu::RenderPipeline {
    let source = compose_material_shader(
        TRANSPARENT_TAIL_FRAME,
        &surface.params_wgsl,
        &surface.source,
        false,
    );
    let module = module(device, "transparent_tail", source);
    let add = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    // What gets through multiplies: dst × (1 − α).
    let through = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::OneMinusSrc,
        operation: wgpu::BlendOperation::Add,
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("transparent_tail"),
        layout: Some(&layouts.material),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_forward"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_tail"),
            targets: &[
                Some(wgpu::ColorTargetState {
                    format: ACCUM_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: add,
                        alpha: add,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: REVEAL_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: through,
                        alpha: through,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ],
            compilation_options: Default::default(),
        }),
        primitive: both_faces(),
        depth_stencil: Some(depth_read(depth_format)),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

pub(super) fn shade(
    device: &wgpu::Device,
    layouts: &Layouts,
    surface: &SurfaceSource,
) -> wgpu::ComputePipeline {
    let source = compose_material_shader(
        TRANSPARENT_SHADE_FRAME,
        &surface.params_wgsl,
        &surface.source,
        false,
    );
    let module = module(device, "transparent_shade", source);
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("transparent_shade"),
        layout: Some(&layouts.material),
        module: &module,
        entry_point: Some("cs_shade"),
        compilation_options: Default::default(),
        cache: None,
    })
}

pub(super) fn composite(device: &wgpu::Device, layouts: &Layouts) -> wgpu::RenderPipeline {
    let module = module(
        device,
        "transparent_composite",
        compose_transparent_composite(),
    );
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("transparent_composite_layout"),
        bind_group_layouts: &[Some(&layouts.composite)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("transparent_composite"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_fullscreen"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_composite"),
            targets: &[Some(wgpu::ColorTargetState {
                format: HDR_COLOR_FORMAT,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::COLOR,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

pub(super) fn args(device: &wgpu::Device, layouts: &Layouts) -> wgpu::ComputePipeline {
    let module = module(
        device,
        "transparent_args",
        TRANSPARENT_ARGS_SHADER.to_owned(),
    );
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("transparent_args_layout"),
        bind_group_layouts: &[Some(&layouts.args)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("transparent_args"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("cs_args"),
        compilation_options: Default::default(),
        cache: None,
    })
}

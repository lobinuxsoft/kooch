//! The masked passes' layouts and pipelines (#452).

use super::MaskedTarget;
use crate::meshlet::vbuf64_stage::{DUMMY_COLOR_FORMAT, VBUF64_FORMAT};

pub(super) const BINS_SHADER: &str = include_str!("../../../shaders/masked_bins.wgsl");

const RASTER_STAGES: wgpu::ShaderStages =
    wgpu::ShaderStages::VERTEX.union(wgpu::ShaderStages::FRAGMENT);
const READ: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: true };
const WRITE: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: false };
const UNIFORM: wgpu::BufferBindingType = wgpu::BufferBindingType::Uniform;

fn buffer(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
    has_dynamic_offset: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset,
            min_binding_size: None,
        },
        count: None,
    }
}

pub(super) struct Layouts {
    /// Camera, per-bin screen, `inti`, the binned slots and their bases; R64 adds its target.
    pub frame: wgpu::BindGroupLayout,
    pub materials: wgpu::BindGroupLayout,
    /// The cull's visible list and the instances, as `surface_reconstruct` reads them.
    pub scene: wgpu::BindGroupLayout,
    pub raster: wgpu::PipelineLayout,
    pub bins: wgpu::BindGroupLayout,
    pub dispatch: wgpu::BindGroupLayout,
    pub prepare: wgpu::ComputePipeline,
    pub count: wgpu::ComputePipeline,
    pub offsets: wgpu::ComputePipeline,
    pub scatter: wgpu::ComputePipeline,
}

impl Layouts {
    pub(super) fn new(
        device: &wgpu::Device,
        target: MaskedTarget,
        meshlet_bgl: &wgpu::BindGroupLayout,
    ) -> Self {
        let vertex = wgpu::ShaderStages::VERTEX;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let mut frame_entries = vec![
            buffer(0, RASTER_STAGES, UNIFORM, false),
            buffer(1, RASTER_STAGES, UNIFORM, true),
            buffer(2, fragment, UNIFORM, false),
            buffer(3, vertex, READ, false),
            buffer(4, vertex, READ, false),
        ];
        if target == MaskedTarget::R64 {
            frame_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: fragment,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::Atomic,
                    format: VBUF64_FORMAT,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            });
        }
        let frame = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("masked_frame_bgl"),
            entries: &frame_entries,
        });
        let materials = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("masked_materials_bgl"),
            entries: &[
                buffer(0, fragment, READ, false),
                buffer(1, fragment, READ, false),
            ],
        });
        let scene = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("masked_scene_bgl"),
            entries: &[
                buffer(0, RASTER_STAGES, READ, false),
                buffer(1, RASTER_STAGES, READ, false),
            ],
        });
        let textures = crate::material::MaterialTexturePool::bind_group_layout(device);
        let raster = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("masked_raster_layout"),
            bind_group_layouts: &[
                Some(&frame),
                Some(meshlet_bgl),
                Some(&materials),
                Some(&scene),
                Some(&textures),
            ],
            immediate_size: 0,
        });

        let compute = wgpu::ShaderStages::COMPUTE;
        let bins = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("masked_bins_bgl"),
            entries: &[
                buffer(0, compute, READ, false),
                buffer(1, compute, READ, false),
                buffer(2, compute, READ, false),
                buffer(3, compute, READ, false),
                buffer(4, compute, WRITE, false),
                buffer(5, compute, WRITE, false),
                buffer(6, compute, WRITE, false),
                buffer(7, compute, WRITE, false),
            ],
        });
        let dispatch = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("masked_dispatch_bgl"),
            entries: &[buffer(0, compute, WRITE, false)],
        });
        let bins_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("masked_bins_layout"),
            bind_group_layouts: &[Some(&bins)],
            immediate_size: 0,
        });
        let prepare_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("masked_prepare_layout"),
            bind_group_layouts: &[Some(&bins), Some(&dispatch)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("masked_bins"),
            source: wgpu::ShaderSource::Wgsl(BINS_SHADER.into()),
        });
        let entry = |name: &str, layout: &wgpu::PipelineLayout| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: Some(layout),
                module: &module,
                entry_point: Some(name),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            prepare: entry("cs_prepare", &prepare_layout),
            count: entry("cs_count", &bins_layout),
            offsets: entry("cs_offsets", &bins_layout),
            scatter: entry("cs_scatter", &bins_layout),
            dispatch,
            frame,
            materials,
            scene,
            raster,
            bins,
        }
    }
}

/// One masked shader's raster: the opaque raster's state, with the material's `surface` deciding
/// which fragments land.
pub(super) fn pipeline(
    device: &wgpu::Device,
    layouts: &Layouts,
    target: MaskedTarget,
    depth_format: wgpu::TextureFormat,
    source: &str,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("masked_raster"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    // R64 writes through its storage binding; the colour target only satisfies the pipeline.
    let colour = match target {
        MaskedTarget::R64 => wgpu::ColorTargetState {
            format: DUMMY_COLOR_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::empty(),
        },
        MaskedTarget::R32 => crate::meshlet::VISIBILITY_BUFFER_FORMAT.into(),
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("masked_raster"),
        layout: Some(&layouts.raster),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_masked"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_masked"),
            targets: &[Some(colour)],
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
            // Reversed-Z, as the opaque raster.
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

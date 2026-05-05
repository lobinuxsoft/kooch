//! `MeshletCull::new` — pipeline + buffer + layout construction.

use std::num::NonZeroU64;

use wgpu::util::DeviceExt;

use crate::meshlet::cull::CullParams;
use crate::meshlet::gpu_meshlet::meshlet_bind_group_layout;
use crate::meshlet::scene::{MeshletScene, SceneCullParams};

use super::types::{DrawIndirectArgs, HiZTestParams};
use super::MeshletCull;

const CULL_SHADER_SOURCE: &str = include_str!("../../../shaders/meshlet_cull.wgsl");

impl MeshletCull {
    /// Creates a dispatcher sized for at most `capacity` visible
    /// meshlets per frame. `max_triangles_per_meshlet` controls the
    /// fixed `vertex_count` used by the indirect draw — must match the
    /// builder's setting (default `meshlet::DEFAULT_MAX_TRIANGLES`).
    pub fn new(
        device: &wgpu::Device,
        capacity: u32,
        max_triangles_per_meshlet: u32,
    ) -> Self {
        assert!(capacity > 0, "MeshletCull capacity must be non-zero");

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet_cull_shader"),
            source: wgpu::ShaderSource::Wgsl(CULL_SHADER_SOURCE.into()),
        });

        let cull_bgl = build_cull_bgl(device);
        let hi_z_bgl = build_hi_z_bgl(device);
        let scene_bgl = MeshletScene::bind_group_layout(device);
        let meshlet_bgl = meshlet_bind_group_layout(device);
        // Two-binding subset of the pool BGL — keeps the cull
        // entry under the wgpu max_storage_buffers_per_shader_stage
        // limit (8). The full 5-binding pool BGL is used by the
        // rasterizer + deferred where storage-buffer headroom is
        // larger and vertex/triangle buffers are needed.
        let pool_bgl = build_cull_pool_bgl(device);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet_cull_pipeline_layout"),
            bind_group_layouts: &[Some(&cull_bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("meshlet_cull_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_cull"),
            compilation_options: Default::default(),
            cache: None,
        });

        let pipeline_layout_hi_z = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet_cull_hi_z_pipeline_layout"),
            bind_group_layouts: &[Some(&cull_bgl), Some(&hi_z_bgl)],
            immediate_size: 0,
        });
        let pipeline_hi_z = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("meshlet_cull_hi_z_pipeline"),
            layout: Some(&pipeline_layout_hi_z),
            module: &shader,
            entry_point: Some("cs_cull_hi_z"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Scene-wide cull pipeline (`cs_cull_scene`) — group(0) cull
        // shared with the per-mesh path, group(2) instance buffer +
        // SceneCullParams. group(1) is unused here so the layout array
        // marks it None.
        let pipeline_layout_scene = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet_cull_scene_pipeline_layout"),
            bind_group_layouts: &[Some(&cull_bgl), None, Some(&scene_bgl)],
            immediate_size: 0,
        });
        let pipeline_scene = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("meshlet_cull_scene_pipeline"),
            layout: Some(&pipeline_layout_scene),
            module: &shader,
            entry_point: Some("cs_cull_scene"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Multi-mesh scene cull (`cs_cull_scene_pool`) — group(0) cull
        // shared with the per-mesh path (the `descriptors` binding at
        // group(0)@1 stays bound but the entry point ignores it),
        // group(1) the GlobalMeshPool, group(2) the scene buffers.
        let pipeline_layout_scene_pool =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meshlet_cull_scene_pool_pipeline_layout"),
                bind_group_layouts: &[Some(&cull_bgl), Some(&pool_bgl), Some(&scene_bgl)],
                immediate_size: 0,
            });
        let pipeline_scene_pool =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("meshlet_cull_scene_pool_pipeline"),
                layout: Some(&pipeline_layout_scene_pool),
                module: &shader,
                entry_point: Some("cs_cull_scene_pool"),
                compilation_options: Default::default(),
                cache: None,
            });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_cull_params"),
            size: std::mem::size_of::<CullParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hi_z_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_cull_hi_z_params"),
            size: std::mem::size_of::<HiZTestParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_cull_scene_params"),
            size: std::mem::size_of::<SceneCullParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let visible_meshlets = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_visible_ids"),
            size: capacity as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let visible_count = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_visible_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let vertex_count_per_instance = max_triangles_per_meshlet * 3;
        let indirect_args = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_indirect_args"),
            contents: bytemuck::bytes_of(&DrawIndirectArgs {
                vertex_count: vertex_count_per_instance,
                instance_count: 0,
                first_vertex: 0,
                first_instance: 0,
            }),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            pipeline,
            pipeline_hi_z,
            pipeline_scene,
            pipeline_scene_pool,
            cull_bgl,
            hi_z_bgl,
            scene_bgl,
            meshlet_bgl,
            pool_bgl,
            params_buffer,
            hi_z_params_buffer,
            scene_params_buffer,
            visible_meshlets,
            visible_count,
            indirect_args,
            capacity,
            vertex_count_per_instance,
        }
    }
}

fn build_cull_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("meshlet_cull_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(
                        std::mem::size_of::<CullParams>() as u64,
                    ),
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
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(4),
                },
                count: None,
            },
        ],
    })
}

/// Two-binding pool BGL the cull pass uses (mesh_descriptors +
/// meshlets). Matches bindings 0-1 of [`crate::meshlet::pool::GpuGlobalMeshPool::bind_group_layout`]
/// so a single pool object can be sliced into either layout via
/// `bind_group_with_layout`-style construction at dispatch time.
fn build_cull_pool_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let read_only = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("meshlet_cull_pool_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: read_only,
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: read_only,
                count: None,
            },
        ],
    })
}

fn build_hi_z_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("meshlet_cull_hi_z_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(
                        std::mem::size_of::<HiZTestParams>() as u64,
                    ),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

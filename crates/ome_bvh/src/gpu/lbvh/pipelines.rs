use std::num::NonZeroU64;

use crate::node::BvhNode;

/// Compiled compute pipelines for the three LBVH passes. Built once
/// per `BvhGpuBuilder`, reused across builds. `pipeline_cache`
/// integration is wired through the constructor.
pub struct LbvhPipelines {
    pub leaves_pipeline: wgpu::ComputePipeline,
    pub leaves_bgl: wgpu::BindGroupLayout,
    pub internal_pipeline: wgpu::ComputePipeline,
    pub internal_bgl: wgpu::BindGroupLayout,
    pub aabb_pipeline: wgpu::ComputePipeline,
    pub aabb_bgl: wgpu::BindGroupLayout,
}

impl LbvhPipelines {
    pub fn new(
        device: &wgpu::Device,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        // -- write_leaves pipeline (5 bindings: nodes, aabbs, indices, done, cfg) --
        let leaves_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::karras_leaves"),
            source: wgpu::ShaderSource::Wgsl(crate::gpu::KARRAS_LEAVES_WGSL.into()),
        });
        let leaves_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::karras_leaves_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<BvhNode>() as u64,
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
                        min_binding_size: NonZeroU64::new(32),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
            ],
        });
        let leaves_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::karras_leaves_pl"),
            bind_group_layouts: &[Some(&leaves_bgl)],
            immediate_size: 0,
        });
        let leaves_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_bvh::karras_leaves_pipeline"),
            layout: Some(&leaves_pl),
            module: &leaves_shader,
            entry_point: Some("write_leaves_main"),
            compilation_options: Default::default(),
            cache: pipeline_cache,
        });

        // -- karras_internal pipeline (5 bindings: nodes, morton, parents, done, cfg) --
        let internal_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::karras_internal"),
            source: wgpu::ShaderSource::Wgsl(crate::gpu::KARRAS_INTERNAL_WGSL.into()),
        });
        let internal_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::karras_internal_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<BvhNode>() as u64,
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
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
            ],
        });
        let internal_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::karras_internal_pl"),
            bind_group_layouts: &[Some(&internal_bgl)],
            immediate_size: 0,
        });
        let internal_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_bvh::karras_internal_pipeline"),
            layout: Some(&internal_pl),
            module: &internal_shader,
            entry_point: Some("karras_internal_main"),
            compilation_options: Default::default(),
            cache: pipeline_cache,
        });

        // -- aabb_propagate pipeline (3 bindings: nodes, done, cfg) --
        let aabb_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::karras_aabb"),
            source: wgpu::ShaderSource::Wgsl(crate::gpu::KARRAS_AABB_WGSL.into()),
        });
        let aabb_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::karras_aabb_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<BvhNode>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
            ],
        });
        let aabb_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::karras_aabb_pl"),
            bind_group_layouts: &[Some(&aabb_bgl)],
            immediate_size: 0,
        });
        let aabb_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_bvh::karras_aabb_pipeline"),
            layout: Some(&aabb_pl),
            module: &aabb_shader,
            entry_point: Some("aabb_propagate_main"),
            compilation_options: Default::default(),
            cache: pipeline_cache,
        });

        Self {
            leaves_pipeline,
            leaves_bgl,
            internal_pipeline,
            internal_bgl,
            aabb_pipeline,
            aabb_bgl,
        }
    }
}

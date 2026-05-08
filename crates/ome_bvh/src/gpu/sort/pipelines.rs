use std::num::NonZeroU64;

/// Owned compute pipelines for the sort. Built once per `BvhGpuBuilder`,
/// reused across builds. `pipeline_cache` integration is wired through
/// the constructor.
pub struct SortPipelines {
    pub init_pipeline: wgpu::ComputePipeline,
    pub init_bgl: wgpu::BindGroupLayout,
    pub histogram_pipeline: wgpu::ComputePipeline,
    pub histogram_bgl: wgpu::BindGroupLayout,
    pub scan_pipeline: wgpu::ComputePipeline,
    pub scan_bgl: wgpu::BindGroupLayout,
    pub scatter_pipeline: wgpu::ComputePipeline,
    pub scatter_bgl: wgpu::BindGroupLayout,
}

impl SortPipelines {
    pub fn new(
        device: &wgpu::Device,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        // -- init pipeline --
        let init_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::sort_init"),
            source: wgpu::ShaderSource::Wgsl(crate::gpu::ONESWEEP_INIT_WGSL.into()),
        });
        let init_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::sort_init_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
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
        let init_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::sort_init_pl"),
            bind_group_layouts: &[Some(&init_bgl)],
            immediate_size: 0,
        });
        let init_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_bvh::sort_init_pipeline"),
            layout: Some(&init_pl),
            module: &init_shader,
            entry_point: Some("init_main"),
            compilation_options: Default::default(),
            cache: pipeline_cache,
        });

        // -- histogram pipeline --
        let histogram_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::sort_histogram"),
            source: wgpu::ShaderSource::Wgsl(crate::gpu::ONESWEEP_HISTOGRAM_WGSL.into()),
        });
        let histogram_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::sort_histogram_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
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
        let histogram_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::sort_histogram_pl"),
            bind_group_layouts: &[Some(&histogram_bgl)],
            immediate_size: 0,
        });
        let histogram_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ome_bvh::sort_histogram_pipeline"),
                layout: Some(&histogram_pl),
                module: &histogram_shader,
                entry_point: Some("count_main"),
                compilation_options: Default::default(),
                cache: pipeline_cache,
            });

        // -- scan pipeline --
        let scan_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::sort_scan"),
            source: wgpu::ShaderSource::Wgsl(crate::gpu::ONESWEEP_EXCLUSIVE_SCAN_WGSL.into()),
        });
        let scan_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::sort_scan_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
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
        let scan_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::sort_scan_pl"),
            bind_group_layouts: &[Some(&scan_bgl)],
            immediate_size: 0,
        });
        let scan_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_bvh::sort_scan_pipeline"),
            layout: Some(&scan_pl),
            module: &scan_shader,
            entry_point: Some("scan_main"),
            compilation_options: Default::default(),
            cache: pipeline_cache,
        });

        // -- scatter pipeline --
        let scatter_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::sort_scatter"),
            source: wgpu::ShaderSource::Wgsl(crate::gpu::ONESWEEP_SCATTER_WGSL.into()),
        });
        // 5 storage + 1 storage atomic + 1 uniform = 7 entries.
        let scatter_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::sort_scatter_bgl"),
            entries: &[
                // 0: keys_in
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
                // 1: values_in
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
                // 2: keys_out
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
                // 3: values_out
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
                // 4: global_histogram (read-only — already prefix-summed)
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
                // 5: partition_descriptors (atomic read/write)
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
                // 6: scatter cfg uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
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
        let scatter_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::sort_scatter_pl"),
            bind_group_layouts: &[Some(&scatter_bgl)],
            immediate_size: 0,
        });
        let scatter_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ome_bvh::sort_scatter_pipeline"),
                layout: Some(&scatter_pl),
                module: &scatter_shader,
                entry_point: Some("scatter_main"),
                compilation_options: Default::default(),
                cache: pipeline_cache,
            });

        Self {
            init_pipeline,
            init_bgl,
            histogram_pipeline,
            histogram_bgl,
            scan_pipeline,
            scan_bgl,
            scatter_pipeline,
            scatter_bgl,
        }
    }
}

//! Compute pipeline + bind group layout factories for the four TLAS
//! Karras passes (morton, leaves, internal, aabb). Extracted from
//! [`super`] so the module-level orchestration stays focused on
//! struct shape and `dispatch_rebuild`.

use std::num::NonZeroU64;

use crate::gpu::types::GpuSceneBounds;
use crate::node::BvhNode;

/// Compile the TLAS pass 0 (Morton encode) pipeline + bind group
/// layout. Bindings mirror `shaders/tlas_morton.wgsl`:
///   0 = chunk_descriptors (R storage, 64-byte stride)
///   1 = scene_bounds (uniform, [`GpuSceneBounds`])
///   2 = mortons (RW storage, u32-per-chunk)
///   3 = config (uniform, `KarrasConfig`)
pub(super) fn build_morton_pipeline(
    device: &wgpu::Device,
    pipeline_cache: Option<&wgpu::PipelineCache>,
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ome_bvh::tlas_morton"),
        source: wgpu::ShaderSource::Wgsl(crate::gpu::TLAS_MORTON_WGSL.into()),
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ome_bvh::tlas_morton_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(
                        std::mem::size_of::<GpuSceneBounds>() as u64,
                    ),
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
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(16),
                },
                count: None,
            },
        ],
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ome_bvh::tlas_morton_pl"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("ome_bvh::tlas_morton_pipeline"),
        layout: Some(&pl),
        module: &shader,
        entry_point: Some("tlas_morton_main"),
        compilation_options: Default::default(),
        cache: pipeline_cache,
    });
    (pipeline, bgl)
}

/// Compile the TLAS pass 2 (leaves) pipeline + its bind group layout.
/// Bindings mirror the spec in `shaders/tlas_leaves.wgsl`:
///   0 = tlas_nodes (RW storage)
///   1 = sorted_indices (R storage)
///   2 = chunk_descriptors (R storage)
///   3 = tlas_done (RW storage)
///   4 = config (uniform `KarrasConfig`)
pub(super) fn build_leaves_pipeline(
    device: &wgpu::Device,
    pipeline_cache: Option<&wgpu::PipelineCache>,
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ome_bvh::tlas_leaves"),
        source: wgpu::ShaderSource::Wgsl(crate::gpu::TLAS_LEAVES_WGSL.into()),
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ome_bvh::tlas_leaves_bgl"),
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
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(64),
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
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ome_bvh::tlas_leaves_pl"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("ome_bvh::tlas_leaves_pipeline"),
        layout: Some(&pl),
        module: &shader,
        entry_point: Some("tlas_leaves_main"),
        compilation_options: Default::default(),
        cache: pipeline_cache,
    });
    (pipeline, bgl)
}

/// Compile the TLAS pass 3 (internal-node Karras) pipeline + bind
/// group layout. Bindings mirror `shaders/tlas_internal.wgsl`:
///   0 = tlas_nodes (RW storage)
///   1 = sorted_morton (R storage; same buffer as `tlas_mortons`
///       post-sort)
///   2 = parents (RW storage, role_idx-keyed)
///   3 = done (RW storage, role_idx-keyed)
///   4 = config (uniform `KarrasConfig`)
pub(super) fn build_internal_pipeline(
    device: &wgpu::Device,
    pipeline_cache: Option<&wgpu::PipelineCache>,
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ome_bvh::tlas_internal"),
        source: wgpu::ShaderSource::Wgsl(crate::gpu::TLAS_INTERNAL_WGSL.into()),
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ome_bvh::tlas_internal_bgl"),
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
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ome_bvh::tlas_internal_pl"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("ome_bvh::tlas_internal_pipeline"),
        layout: Some(&pl),
        module: &shader,
        entry_point: Some("tlas_internal_main"),
        compilation_options: Default::default(),
        cache: pipeline_cache,
    });
    (pipeline, bgl)
}

/// Compile the TLAS pass 4 (AABB propagation) pipeline + bind group
/// layout. Bindings mirror `shaders/tlas_aabb.wgsl`:
///   0 = tlas_nodes (RW storage)
///   1 = parents (R storage, role_idx-keyed; bound for symmetry,
///       unused in the multi-dispatch convergence variant)
///   2 = done (RW storage, role_idx-keyed)
///   3 = config (uniform `KarrasConfig`)
pub(super) fn build_aabb_pipeline(
    device: &wgpu::Device,
    pipeline_cache: Option<&wgpu::PipelineCache>,
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ome_bvh::tlas_aabb"),
        source: wgpu::ShaderSource::Wgsl(crate::gpu::TLAS_AABB_WGSL.into()),
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ome_bvh::tlas_aabb_bgl"),
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
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(16),
                },
                count: None,
            },
        ],
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ome_bvh::tlas_aabb_pl"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("ome_bvh::tlas_aabb_pipeline"),
        layout: Some(&pl),
        module: &shader,
        entry_point: Some("tlas_aabb_propagate_main"),
        compilation_options: Default::default(),
        cache: pipeline_cache,
    });
    (pipeline, bgl)
}

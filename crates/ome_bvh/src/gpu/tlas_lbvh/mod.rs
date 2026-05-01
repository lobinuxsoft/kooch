//! TLAS Karras LBVH GPU rebuild pipeline (epic #370 PR-1).
//!
//! Mirrors the BLAS [`crate::gpu::lbvh`] pipeline at the algorithmic
//! level — same Karras 2012 algorithm, same workgroup size for the
//! tree-construction passes, same `KarrasConfig` uniform layout — but
//! the leaves are *chunk descriptors* (one per resident chunk) instead
//! of per-primitive AABBs. Output: a flat `2N - 1` [`BvhNode`] array
//! in [`crate::accel::buffers::AccelBuffers::tlas_nodes`] byte-identical
//! to what the legacy CPU [`crate::accel::tlas::rebuild`] used to
//! produce.
//!
//! This module lands incrementally across PR-1 commits. Current state
//! (commit 4): Morton encode + onesweep radix sort wired. Karras
//! tree-construction passes (leaves / internal / aabb) follow.

mod dispatch;

#[cfg(test)]
mod tests;

use std::num::NonZeroU64;

use super::karras_common::KarrasConfig;
use super::sort::{SortBuffers, SortPipelines};
use super::sort_types::ITEMS_PER_TILE;
use super::types::GpuSceneBounds;
use crate::node::BvhNode;

/// Initial scratch capacity for the TLAS onesweep sort. Matches the
/// `INITIAL_LBVH_CAPACITY` of the BLAS pipeline so both sides of the
/// pool grow on the same power-of-two ladder.
const INITIAL_TLAS_SORT_CAPACITY: u64 = 1024;

/// Compiled TLAS rebuild pipelines + their uniform staging buffers.
/// Shared across every rebuild dispatch on a given device; safe to
/// reuse from frame to frame because every per-rebuild input
/// (chunk count, scene bounds, scratch buffers) is passed by parameter.
pub struct TlasGpuBuilder {
    pub morton_pipeline: wgpu::ComputePipeline,
    pub morton_bgl: wgpu::BindGroupLayout,
    /// Uniform buffer holding [`GpuSceneBounds`] for the current
    /// rebuild. Written via `queue.write_buffer` at dispatch time.
    pub scene_bounds_buffer: wgpu::Buffer,
    /// Uniform buffer holding the Karras `n` (live chunk count). Same
    /// layout as the BLAS [`crate::gpu::lbvh::LbvhConfig`] so the
    /// later TLAS Karras passes can share it.
    pub config_buffer: wgpu::Buffer,
    /// Onesweep radix sort pipelines — reused as-is from the BLAS path.
    /// Sorting at TLAS scale (≤ `caps.max_chunks`) reuses the same
    /// shaders and the same per-pass uniform layout; only the input /
    /// output buffer plumbing differs.
    pub sort_pipelines: SortPipelines,
    /// Onesweep scratch buffers (`keys_a/b`, `values_a/b`, histogram,
    /// partition descriptors). Pre-allocated to
    /// [`INITIAL_TLAS_SORT_CAPACITY`] so the rebuild dispatch never
    /// realloc's; grown on demand if the chunk pool ever exceeds it.
    pub sort_buffers: SortBuffers,
    /// Pass 2 — TLAS leaves writer. Lays down N leaf nodes encoded
    /// with `right_or_count = chunk_idx | BVH_LEAF_FLAG`.
    pub leaves_pipeline: wgpu::ComputePipeline,
    pub leaves_bgl: wgpu::BindGroupLayout,
    /// Pass 3 — TLAS internal-node Karras constructor. Builds the
    /// N-1 internals via delta + range + split, writes parent
    /// pointers with the TLAS-specific role_idx convention.
    pub internal_pipeline: wgpu::ComputePipeline,
    pub internal_bgl: wgpu::BindGroupLayout,
}

impl TlasGpuBuilder {
    pub fn new(
        device: &wgpu::Device,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let morton_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::tlas_morton"),
            source: wgpu::ShaderSource::Wgsl(super::TLAS_MORTON_WGSL.into()),
        });
        let morton_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
        let morton_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::tlas_morton_pl"),
            bind_group_layouts: &[Some(&morton_bgl)],
            immediate_size: 0,
        });
        let morton_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_bvh::tlas_morton_pipeline"),
            layout: Some(&morton_pl),
            module: &morton_shader,
            entry_point: Some("tlas_morton_main"),
            compilation_options: Default::default(),
            cache: pipeline_cache,
        });

        let scene_bounds_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::tlas_scene_bounds"),
            size: std::mem::size_of::<GpuSceneBounds>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let config_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::tlas_config"),
            size: std::mem::size_of::<KarrasConfig>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let sort_pipelines = SortPipelines::new(device, pipeline_cache);
        let mut sort_buffers = SortBuffers::new(device);
        let initial_partitions = partitions_for(INITIAL_TLAS_SORT_CAPACITY as u32);
        sort_buffers.ensure_capacity(device, INITIAL_TLAS_SORT_CAPACITY, initial_partitions);

        let (leaves_pipeline, leaves_bgl) = build_leaves_pipeline(device, pipeline_cache);
        let (internal_pipeline, internal_bgl) =
            build_internal_pipeline(device, pipeline_cache);

        Self {
            morton_pipeline,
            morton_bgl,
            scene_bounds_buffer,
            config_buffer,
            sort_pipelines,
            sort_buffers,
            leaves_pipeline,
            leaves_bgl,
            internal_pipeline,
            internal_bgl,
        }
    }

    /// Grow the onesweep scratch if the chunk pool exceeds the current
    /// capacity. Idempotent — does nothing when `n_chunks <= capacity`.
    pub fn ensure_capacity(&mut self, device: &wgpu::Device, n_chunks: u64) {
        let partitions = partitions_for(n_chunks as u32);
        self.sort_buffers
            .ensure_capacity(device, n_chunks, partitions);
    }
}

/// Number of onesweep partitions for `count` keys. Mirrors the formula
/// `dispatch_sort_into` uses internally so the caller's
/// `ensure_capacity` request always matches the dispatch's needs.
fn partitions_for(count: u32) -> u32 {
    count.div_ceil(ITEMS_PER_TILE)
}

/// Compile the TLAS pass 2 (leaves) pipeline + its bind group layout.
/// Bindings mirror the spec in `shaders/tlas_leaves.wgsl`:
///   0 = tlas_nodes (RW storage)
///   1 = sorted_indices (R storage)
///   2 = chunk_descriptors (R storage)
///   3 = tlas_done (RW storage)
///   4 = config (uniform `KarrasConfig`)
fn build_leaves_pipeline(
    device: &wgpu::Device,
    pipeline_cache: Option<&wgpu::PipelineCache>,
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ome_bvh::tlas_leaves"),
        source: wgpu::ShaderSource::Wgsl(super::TLAS_LEAVES_WGSL.into()),
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
fn build_internal_pipeline(
    device: &wgpu::Device,
    pipeline_cache: Option<&wgpu::PipelineCache>,
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ome_bvh::tlas_internal"),
        source: wgpu::ShaderSource::Wgsl(super::TLAS_INTERNAL_WGSL.into()),
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


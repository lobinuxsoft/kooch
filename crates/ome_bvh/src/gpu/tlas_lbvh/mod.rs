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
//! `dispatch_rebuild` chains the five passes (morton → sort → leaves
//! → internal → aabb) into a single encoder submission. The caller
//! owns the encoder so the rebuild can co-batch with renderer work.

mod dispatch;
mod pipelines;

#[cfg(test)]
mod tests;

use crate::accel::buffers::AccelBuffers;
use crate::accel::descriptor::ChunkDescriptor;
use crate::aabb::Aabb;

use super::karras_common::KarrasConfig;
use super::sort::{SortBuffers, SortPipelines};
use super::sort_types::ITEMS_PER_TILE;
use super::types::GpuSceneBounds;

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
    pub sort_pipelines: SortPipelines,
    /// Onesweep scratch buffers (`keys_a/b`, `values_a/b`, histogram,
    /// partition descriptors). Pre-allocated to
    /// [`INITIAL_TLAS_SORT_CAPACITY`] so the rebuild dispatch never
    /// realloc's; grown on demand if the chunk pool ever exceeds it.
    pub sort_buffers: SortBuffers,
    /// Pass 2 — TLAS leaves writer.
    pub leaves_pipeline: wgpu::ComputePipeline,
    pub leaves_bgl: wgpu::BindGroupLayout,
    /// Pass 3 — TLAS internal-node Karras constructor.
    pub internal_pipeline: wgpu::ComputePipeline,
    pub internal_bgl: wgpu::BindGroupLayout,
    /// Pass 4 — TLAS bottom-up AABB propagation.
    pub aabb_pipeline: wgpu::ComputePipeline,
    pub aabb_bgl: wgpu::BindGroupLayout,
}

impl TlasGpuBuilder {
    pub fn new(
        device: &wgpu::Device,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let (morton_pipeline, morton_bgl) =
            pipelines::build_morton_pipeline(device, pipeline_cache);
        let (leaves_pipeline, leaves_bgl) =
            pipelines::build_leaves_pipeline(device, pipeline_cache);
        let (internal_pipeline, internal_bgl) =
            pipelines::build_internal_pipeline(device, pipeline_cache);
        let (aabb_pipeline, aabb_bgl) =
            pipelines::build_aabb_pipeline(device, pipeline_cache);

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
            aabb_pipeline,
            aabb_bgl,
        }
    }

    /// Grow the onesweep scratch if the chunk pool exceeds the current
    /// capacity. Idempotent — does nothing when `n_chunks <= capacity`.
    pub fn ensure_capacity(&mut self, device: &wgpu::Device, n_chunks: u64) {
        let partitions = partitions_for(n_chunks as u32);
        self.sort_buffers
            .ensure_capacity(device, n_chunks, partitions);
    }

    /// End-to-end TLAS rebuild — chains every pass into the caller's
    /// encoder so morton + sort + leaves + internal + aabb share a
    /// single submission and a single CPU side-effect window. Cero
    /// CPU readback in the dispatch path (commit 7's no-readback
    /// invariant; verified by `tlas_gpu_dispatch_no_readback` in
    /// commit 10).
    ///
    /// `cpu_descriptors` is the CPU mirror of every live chunk's
    /// `ChunkDescriptor`, used only to fold the scene-wide
    /// [`GpuSceneBounds`] for the morton normalisation. The shader
    /// reads chunk descriptors from `accel_buffers.chunk_descriptors`,
    /// so the caller is responsible for keeping that GPU buffer in
    /// sync with `cpu_descriptors` (already the case in the streaming
    /// path — `OmeAccel::insert_chunk` writes both).
    ///
    /// Edge cases:
    /// - `n == 0`: no-op. Caller (`accel::tlas::rebuild` in commit 8)
    ///   handles the sentinel zero-write.
    /// - `n == 1`: leaves pass writes the single leaf at
    ///   `tlas_nodes[0]` (the `select(N - 1, 0, N == 0)` branch in
    ///   `tlas_leaves.wgsl` covers this); internal + aabb passes are
    ///   skipped because there is no topology to build.
    pub fn dispatch_rebuild(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        accel_buffers: &AccelBuffers,
        cpu_descriptors: &[ChunkDescriptor],
        n: u32,
    ) {
        if n == 0 {
            return;
        }

        let aabbs: Vec<Aabb> = cpu_descriptors
            .iter()
            .map(|d| Aabb::new(d.aabb_min.into(), d.aabb_max.into()))
            .collect();
        let scene = GpuSceneBounds::from_aabbs(&aabbs);

        self.dispatch_morton(
            device,
            queue,
            encoder,
            &accel_buffers.chunk_descriptors,
            &accel_buffers.tlas_mortons,
            scene,
            n,
        );
        self.dispatch_sort(
            device,
            queue,
            encoder,
            &accel_buffers.tlas_mortons,
            &accel_buffers.tlas_sorted_indices,
            n,
        );
        self.dispatch_leaves(
            device,
            encoder,
            &accel_buffers.tlas_nodes,
            &accel_buffers.tlas_sorted_indices,
            &accel_buffers.chunk_descriptors,
            &accel_buffers.tlas_done,
            n,
        );
        if n >= 2 {
            self.dispatch_internal(
                device,
                encoder,
                &accel_buffers.tlas_nodes,
                &accel_buffers.tlas_mortons,
                &accel_buffers.tlas_parents,
                &accel_buffers.tlas_done,
                n,
            );
            self.dispatch_aabb(
                device,
                encoder,
                &accel_buffers.tlas_nodes,
                &accel_buffers.tlas_parents,
                &accel_buffers.tlas_done,
                n,
            );
        }
    }
}

/// Number of onesweep partitions for `count` keys. Mirrors the formula
/// `dispatch_sort_into` uses internally so the caller's
/// `ensure_capacity` request always matches the dispatch's needs.
fn partitions_for(count: u32) -> u32 {
    count.div_ceil(ITEMS_PER_TILE)
}

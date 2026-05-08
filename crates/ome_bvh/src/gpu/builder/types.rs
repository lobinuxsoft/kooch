//! Public struct definitions for the GPU builder: [`BvhGpuBuilder`]
//! owns the compute pipelines and reusable buffers; [`BvhTimestamps`]
//! holds the per-pass timestamp query set and resolve buffers.

use crate::gpu::lbvh::{LbvhBuffers, LbvhPipelines};
use crate::gpu::sort::{SortBuffers, SortPipelines};

/// Owns the GPU compute infrastructure for the LBVH build pipeline.
///
/// Built once per app instance, reused across every BVH build. Hold
/// it in a long-lived resource (e.g. a `wgpu::Device`-scoped struct
/// in `ome_render` or as a `Resources` entry).
pub struct BvhGpuBuilder {
    pub(crate) morton_pipeline: wgpu::ComputePipeline,
    pub(crate) morton_bgl: wgpu::BindGroupLayout,

    // Reusable buffers — capacities are tracked separately so we only
    // grow (never shrink). All buffers use `STORAGE | COPY_DST`; the
    // staging readback path uses `MAP_READ | COPY_DST`.
    pub(crate) aabbs_buffer: wgpu::Buffer,
    pub(crate) aabbs_capacity: u64,

    pub(crate) scene_bounds_buffer: wgpu::Buffer,

    pub(crate) morton_codes_buffer: wgpu::Buffer,
    pub(crate) morton_codes_capacity: u64,

    /// Onesweep radix sort pipelines + reusable buffers. Driven by
    /// [`Self::dispatch_sort_into`] (pass-through wrapper around
    /// [`crate::gpu::sort::dispatch_sort_into`]).
    pub(crate) sort_pipelines: SortPipelines,
    pub(crate) sort_buffers: SortBuffers,

    /// Karras LBVH constructor pipelines + reusable buffers. Driven by
    /// [`Self::dispatch_lbvh_into`] (pass-through wrapper around
    /// [`crate::gpu::lbvh::dispatch_lbvh_build`]).
    pub(crate) lbvh_pipelines: LbvhPipelines,
    pub(crate) lbvh_buffers: LbvhBuffers,

    /// Timestamp query set + resolve buffer for per-pass profiling.
    /// `None` when the device was not built with
    /// [`wgpu::Features::TIMESTAMP_QUERY`] +
    /// [`wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES`] — the engine
    /// requests these as **optional** features in
    /// [`ome_core::gpu`](../../../../ome_core/gpu/index.html), so the
    /// builder must stay correct on adapters that don't expose them.
    /// Without timestamps the builder skips both `create_query_set` and
    /// the `timestamp_writes` on the morton pass; the validation error
    /// otherwise rejects the entire submission and silently corrupts
    /// downstream `done_buffer` reads (#333).
    pub(crate) timestamps: Option<BvhTimestamps>,
}

/// Per-build timestamp infrastructure. Resolved into `resolve_buffer`
/// which can be mapped + read for per-pass timing.
pub struct BvhTimestamps {
    pub query_set: wgpu::QuerySet,
    pub resolve_buffer: wgpu::Buffer,
    pub readback_buffer: wgpu::Buffer,
    /// `period_ns` reported by the queue. Multiply raw timestamp
    /// deltas by this to get nanoseconds.
    pub period_ns: f32,
}

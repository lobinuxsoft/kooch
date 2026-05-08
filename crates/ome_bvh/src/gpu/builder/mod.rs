//! [`BvhGpuBuilder`] — owns the GPU compute pipelines and reusable
//! buffers for the LBVH build (Morton encoding → onesweep radix sort
//! → Karras parallel construction).
//!
//! Single source of truth for state that survives across builds:
//! pipelines (cached via the optional [`wgpu::PipelineCache`] handle),
//! storage buffers (grow-on-demand, never realloc per build), and the
//! timestamp query set (per-pass profiling). The `Bvh::build_gpu` API
//! threads its work through this struct.
//!
//! Designed for **production use day one** (per session 2026-04-26
//! quater feedback): `wgpu::PipelineCache` integration, reusable
//! buffers, async API surface, timestamp query instrumentation.

mod bvh_gpu_builder;
mod timestamps;
mod types;

pub use types::{BvhGpuBuilder, BvhTimestamps};

/// Number of timestamp slots: one pair (start/end) per compute pass.
/// Order in the buffer:
///   `[morton_start, morton_end, sort_start, sort_end, build_start, build_end]`.
pub(super) const TIMESTAMP_QUERY_COUNT: u32 = 6;

/// Per-pass timestamp slot indices. Used by every dispatch helper to
/// write the same query set positions consistently.
pub(super) const TS_MORTON_START: u32 = 0;
#[allow(dead_code)]
pub(super) const TS_MORTON_END: u32 = 1;
#[allow(dead_code)]
pub(super) const TS_SORT_START: u32 = 2;
#[allow(dead_code)]
pub(super) const TS_SORT_END: u32 = 3;
#[allow(dead_code)]
pub(super) const TS_BUILD_START: u32 = 4;
#[allow(dead_code)]
pub(super) const TS_BUILD_END: u32 = 5;

/// Workgroup size for the Morton compute shader. Matches the
/// `@workgroup_size(256)` declaration in `morton.wgsl`. Conservative
/// for portability — RDNA 4 / RTX 4070 happily run 256.
pub(super) const MORTON_WORKGROUP_SIZE: u32 = 256;

/// Initial capacity for storage buffers (in items). Grows by
/// `next_power_of_two` when an upload exceeds capacity.
pub(super) const INITIAL_AABB_CAPACITY: u64 = 256;
pub(super) const INITIAL_MORTON_CAPACITY: u64 = 256;

#[cfg(test)]
pub(crate) mod test_device {
    use std::sync::OnceLock;

    // Mesa radv SIGSEGVs when many threads concurrently call
    // `wgpu::Instance::default().request_adapter(...)`. Acquire once
    // per test binary and clone handles for every call.
    static SHARED: OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = OnceLock::new();

    /// Acquire a wgpu device + queue for unit tests. Picks any
    /// available adapter (vulkan / metal / dx12 / gl). Returns `None`
    /// when no GPU is available — the test in that case skips itself
    /// rather than failing (CI without a display falls into this path).
    ///
    /// `TIMESTAMP_QUERY` and `TIMESTAMP_QUERY_INSIDE_PASSES` features
    /// are requested unconditionally — the BVH builder writes timestamps
    /// in every dispatch and would fail to validate a pipeline without
    /// them. Adapters that don't expose the feature are skipped.
    pub fn try_acquire() -> Option<(wgpu::Device, wgpu::Queue)> {
        SHARED
            .get_or_init(|| {
                pollster::block_on(async {
                    let instance = wgpu::Instance::default();
                    let adapter = instance
                        .request_adapter(&wgpu::RequestAdapterOptions::default())
                        .await
                        .ok()?;

                    let supports_ts =
                        adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
                    let supports_ts_inside = adapter
                        .features()
                        .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES);
                    if !supports_ts || !supports_ts_inside {
                        return None;
                    }

                    let (device, queue) = adapter
                        .request_device(&wgpu::DeviceDescriptor {
                            label: Some("ome_bvh::test_device"),
                            required_features: wgpu::Features::TIMESTAMP_QUERY
                                | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES,
                            required_limits: wgpu::Limits::default(),
                            memory_hints: wgpu::MemoryHints::Performance,
                            trace: wgpu::Trace::Off,
                            experimental_features: wgpu::ExperimentalFeatures::default(),
                        })
                        .await
                        .ok()?;
                    Some((device, queue))
                })
            })
            .clone()
    }
}

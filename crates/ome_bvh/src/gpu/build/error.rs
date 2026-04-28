//! [`BvhBuildError`] — terminal failure modes of the GPU build /
//! refit pipelines. All variants are unrecoverable; the caller drops
//! the handle and either retries the build or fails loudly.

/// Errors surfaced by the GPU build pipeline. All are terminal — the
/// caller should drop the [`super::BvhGpuBuild`] / [`super::BvhGpuRefit`]
/// and either retry or fail loudly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BvhBuildError {
    /// `wgpu::Device` was lost (driver crash, surface reconfigured
    /// mid-build, etc). The build's submission can't complete and the
    /// staging buffers will never resolve.
    DeviceLost,
    /// `map_async` callback reported a buffer mapping failure on either
    /// the nodes or the sorted-indices staging buffer.
    BufferMapFailed,
    /// Buffer allocation failed during `ensure_capacity` — the GPU is
    /// out of memory or the requested size exceeded
    /// `max_buffer_size`. The build was abandoned before submission.
    OutOfMemory,
}

impl std::fmt::Display for BvhBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeviceLost => f.write_str("wgpu device was lost during the BVH build"),
            Self::BufferMapFailed => f.write_str("staging buffer map_async failed"),
            Self::OutOfMemory => f.write_str("GPU buffer allocation failed (out of memory)"),
        }
    }
}

impl std::error::Error for BvhBuildError {}

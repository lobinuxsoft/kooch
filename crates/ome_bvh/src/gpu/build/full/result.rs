//! [`BvhGpuBuildResult<T>`] — mapped readback payload returned by
//! [`super::BvhGpuBuild::poll`] once both staging buffers resolve.

use crate::bvh::Bvh;

/// Result of a successfully resolved [`super::BvhGpuBuild::poll`]. Carries
/// the byte-identical CPU mirror of the GPU build alongside the
/// `sorted_indices` permutation the readback already paid for. The
/// permutation feeds [`Bvh::refit_in_place`] so a CPU consumer
/// (physics broadphase, debug tooling, ...) can stay in sync with
/// subsequent refits over the same topology without a fresh nodes
/// readback.
pub struct BvhGpuBuildResult<T: Copy> {
    pub bvh: Bvh<T>,
    /// `sorted_indices[k]` = original-input position of the leaf at
    /// sorted position `k` (i.e. `nodes[(N-1) + k]`). Empty for the
    /// `n == 0` build path.
    pub sorted_indices: Vec<u32>,
}

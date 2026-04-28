//! GPU-driven frustum culling consumer of the engine-shared BVH.
//!
//! [`FrustumCull`] dispatches a compute pass over
//! [`SharedBvhState::current_leaf_aabbs`] and writes a
//! [`DrawIndexedIndirectArgs`] entry per leaf (in original input
//! order). Each entry's `instance_count` is `1` for visible leaves
//! and `0` for culled or non-mesh leaves; mesh passes consume the
//! buffer via `draw_indexed_indirect` and the GPU command processor
//! skips the zero-instance entries with no shader work.
//!
//! # Why GPU-driven
//!
//! Frustum cull is the cleanest planet-scale-grade GPU consumer of
//! the BVH: per-frame readback of "which N of the M scene primitives
//! are visible" is exactly the round-trip the planet-scale constraint
//! forbids. The shader writes indirect commands that the next mesh
//! pass reads on the GPU; the CPU only ever writes the frustum
//! uniform once per camera change.
//!
//! # Module layout
//!
//! - [`cull`] — `FrustumCull` resource + `FrustumUniforms` /
//!   `DrawIndexedIndirectArgs` POD layouts + the compute dispatch.
//! - `tests` — GPU correctness tests gated on `test_device::try_acquire`.

mod cull;

#[cfg(test)]
mod bench;
#[cfg(test)]
mod tests;

pub use cull::{DrawIndexedIndirectArgs, FrustumCull, FrustumPlanes, FrustumUniforms};

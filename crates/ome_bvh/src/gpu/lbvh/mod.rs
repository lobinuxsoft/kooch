//! Karras LBVH constructor — three-pass GPU pipeline that turns a
//! sorted Morton + AABB list into a 2N-1 [`BvhNode`] array
//! byte-identical to the CPU [`Bvh::build`] output.
//!
//! Passes (each is its own dispatch on the same encoder):
//!
//! 1. **`write_leaves`** (`karras_leaves.wgsl`) — N threads, each
//!    writes its leaf into `nodes[(N-1) + k]`. Reads
//!    `original_aabbs[sorted_indices[k]]` (Morton-permuted lookup).
//!    Sets `done[leaf_idx] = 1` so the propagation pass knows the
//!    leaf's AABB is finalised.
//! 2. **`karras_internal`** (`karras_internal.wgsl`) — N-1 threads,
//!    one per internal node. Each runs Karras' algorithm on the
//!    sorted Morton codes to determine its range, split, and
//!    children, then writes `nodes[i]`, the parent pointers for its
//!    two children, and `done[i] = 0`.
//! 3. **`aabb_propagate`** (`karras_aabb.wgsl`) — looped on the host
//!    once per tree level (⌈log₂ N⌉ + slack iterations). Each
//!    dispatch finalises every internal whose children were finalised
//!    in earlier dispatches. The dispatch boundary is the only
//!    portable cross-workgroup memory barrier in WGSL — atomics give
//!    ordering on the atomic itself but not on adjacent memory, so
//!    the single-dispatch atomic-counter approach Karras' CUDA
//!    implementation uses (relying on `__threadfence`) does not work
//!    portably.
//!
//! All three pipelines are owned by [`LbvhPipelines`]; reusable GPU
//! buffers live in [`LbvhBuffers`]. The high-level orchestration
//! [`dispatch_lbvh_build`] wires them together with the existing
//! Morton + onesweep sort outputs.

mod buffers;
mod dispatch;
mod pipelines;

#[cfg(test)]
mod testing;

/// Re-export so callers and downstream tests keep using
/// `crate::gpu::lbvh::aabb_iterations` (BLAS-flavoured spelling) while
/// the implementation lives in [`super::karras_common`].
pub(crate) use super::karras_common::aabb_iterations;

/// Initial capacity for the LBVH buffers (in leaves count). Grows by
/// `next_power_of_two` when an upload exceeds capacity.
const INITIAL_LBVH_CAPACITY: u64 = 1024;

/// Uniform configuration for every Karras BLAS pass. Re-exported alias
/// of [`super::karras_common::KarrasConfig`] so the `LbvhConfig` name
/// stays available to the existing public API and the BLAS-side
/// shaders that declare `struct LbvhConfig`.
pub type LbvhConfig = super::karras_common::KarrasConfig;

pub use buffers::LbvhBuffers;
pub use dispatch::{
    dispatch_lbvh_aabb_only_into, dispatch_lbvh_build, dispatch_lbvh_internal_and_aabb_into,
    dispatch_lbvh_leaves_into,
};
pub use pipelines::LbvhPipelines;

#[cfg(test)]
pub use testing::readback_nodes_for_test;

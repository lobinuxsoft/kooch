//! [`BvhGpuBuild<T>`] + [`build_gpu`] — full Karras GPU LBVH path.
//!
//! Chains morton + onesweep sort + Karras (leaves + internal + AABB
//! propagation) on a single command encoder, submits, and returns a
//! poll-driven handle. The `n == 0` and `n == 1` branches are made
//! visible at the call site so the orchestrator's edge-case guards
//! stay reviewable.
//!
//! # Module layout
//!
//! - [`result`] — `BvhGpuBuildResult<T>` (mapped readback payload).
//! - [`build`] — `BvhGpuBuild<T>` (handle + `poll` / `gpu_handle` /
//!   `block_on`).
//! - [`orchestrator`] — `build_gpu` (free function chaining morton +
//!   sort + LBVH on a single encoder).
//! - [`empty`] — `empty_build` (`n == 0` placeholder constructor,
//!   crate-private).

mod build;
mod empty;
mod orchestrator;
mod result;

pub use build::BvhGpuBuild;
pub use orchestrator::build_gpu;
pub use result::BvhGpuBuildResult;

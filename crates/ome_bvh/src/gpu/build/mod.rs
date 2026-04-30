//! High-level GPU LBVH build orchestrator.
//!
//! [`Bvh::build_gpu`](crate::Bvh::build_gpu) chains morton + onesweep
//! sort + Karras LBVH on a single command encoder, submits, and
//! returns a [`BvhGpuBuild<T>`] handle. The handle is poll-driven
//! ([`BvhGpuBuild::poll`]) so the caller can integrate it into a
//! frame loop without ever calling `block_on` from the hot path.
//!
//! Two consumption modes are supported:
//!
//! - **CPU readback** (tests, tooling, oneshot tools): the caller polls
//!   until [`BvhGpuBuild::poll`] returns `Some(Ok(Bvh<T>))`, recovering
//!   the flat `Vec<BvhNode>` and the permuted `Vec<T>` payload.
//! - **GPU-resident handoff** (production hot loop): the caller does
//!   NOT readback; it grabs [`BvhGpuBuild::gpu_handle`] for downstream
//!   traversal kernels.
//!
//! [`refit_gpu`] is the symmetric fast path: rewrites only leaves +
//! propagates internals over an already-built topology. Skips morton
//! / sort / Karras' internal-node construction. Returns a
//! [`BvhGpuRefit`] with the same poll-driven lifecycle.
//!
//! # Module layout
//!
//! - [`error`] — `BvhBuildError` (terminal failure modes).
//! - [`lifecycle`] — `MapState`, `GpuBvhHandle` (shared infra used by
//!   both build and refit handles).
//! - [`full`] — `BvhGpuBuild<T>` + `build_gpu` (the full Karras
//!   pipeline path).
//! - [`refit`] — `BvhGpuRefit` + `refit_gpu` (the topology-preserving
//!   fast path).
//! - [`tests`] — golden CPU/GPU consistency + GPU/CPU-ground-truth
//!   refit tests.

pub mod error;
mod full;
mod lifecycle;
mod refit;
mod seed_dump;

pub use error::BvhBuildError;
pub use full::{BvhGpuBuild, BvhGpuBuildResult, build_gpu};
pub use lifecycle::GpuBvhHandle;
pub use refit::{BvhGpuRefit, refit_gpu};

#[cfg(test)]
mod aabb_convergence_tests;

#[cfg(test)]
mod tests;

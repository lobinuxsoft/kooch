//! GPU compute pipeline for the LBVH build.
//!
//! Three passes wired through [`BvhGpuBuilder`]:
//!
//! 1. **Morton encoding** (this PR) — every AABB's centre normalised
//!    against the scene bounds and packed as a 30-bit code in `u32`.
//! 2. **Onesweep radix sort** (PR-3 subtask 2) — sort by Morton code.
//! 3. **Karras LBVH constructor** (PR-3 subtask 3) — flat
//!    `Vec<BvhNode>` byte-identical to the CPU build.
//!
//! Every byte-level layout decision matches the WGSL structs
//! (`std430` clean, no internal padding surprises). The CPU path
//! (`Bvh::build`) and the GPU path (`Bvh::build_gpu`, PR-3 subtask 4)
//! emit byte-identical results — the consistency tests in each
//! pass module verify this against random inputs.

pub mod builder;
pub mod morton;
pub mod types;

pub use builder::{BvhGpuBuilder, BvhTimestamps};
pub use types::{GpuAabb, GpuSceneBounds};

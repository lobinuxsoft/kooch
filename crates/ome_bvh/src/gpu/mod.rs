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
pub mod sort_types;
pub mod types;

pub use builder::{BvhGpuBuilder, BvhTimestamps};
pub use sort_types::{
    FLAG_AGGREGATE, FLAG_INVALID, FLAG_PREFIX, ITEMS_PER_TILE, OnesweepConfig, RADIX_BITS,
    RADIX_BUCKETS, RADIX_PASSES, SORT_WORKGROUP_SIZE, global_histogram_size_bytes,
    partition_descriptors_size_bytes,
};
pub use types::{GpuAabb, GpuSceneBounds};

/// Onesweep init compute shader source. Cleared by the `BvhGpuBuilder`
/// at the start of every sort run.
pub(crate) const ONESWEEP_INIT_WGSL: &str = include_str!("../../shaders/onesweep_init.wgsl");

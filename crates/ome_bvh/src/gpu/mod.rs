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

pub mod build;
pub mod builder;
pub mod karras_common;
pub mod lbvh;
pub mod morton;
pub mod sort;
pub mod sort_types;
pub mod tlas_lbvh;
pub mod types;

pub use build::{
    BvhBuildError, BvhGpuBuild, BvhGpuBuildResult, BvhGpuRefit, GpuBvhHandle, build_gpu, refit_gpu,
};
pub use builder::{BvhGpuBuilder, BvhTimestamps};
pub use lbvh::{
    LbvhBuffers, LbvhConfig, LbvhPipelines, dispatch_lbvh_aabb_only_into, dispatch_lbvh_build,
};
pub use sort_types::{
    FLAG_AGGREGATE, FLAG_INVALID, FLAG_PREFIX, ITEMS_PER_TILE, OnesweepConfig, RADIX_BITS,
    RADIX_BUCKETS, RADIX_PASSES, SORT_WORKGROUP_SIZE, global_histogram_size_bytes,
    partition_descriptors_size_bytes,
};
pub use types::{GpuAabb, GpuSceneBounds};

/// Onesweep init compute shader source. Cleared by the `BvhGpuBuilder`
/// at the start of every sort run.
pub(crate) const ONESWEEP_INIT_WGSL: &str = include_str!("../../shaders/onesweep_init.wgsl");

/// Onesweep global histogram count shader source. One dispatch over
/// the input keys — counts all 4 passes' digits simultaneously via
/// workgroup-local histograms flushed atomically to global.
pub(crate) const ONESWEEP_HISTOGRAM_WGSL: &str =
    include_str!("../../shaders/onesweep_global_histogram.wgsl");

/// Onesweep exclusive scan shader source. One dispatch per pass (4
/// total) — converts each pass's 256-bucket histogram from counts
/// into exclusive prefix sums (bucket starting offsets in the sorted
/// output).
pub(crate) const ONESWEEP_EXCLUSIVE_SCAN_WGSL: &str =
    include_str!("../../shaders/onesweep_exclusive_scan.wgsl");

/// Onesweep scatter shader source. One dispatch per pass (4 total) —
/// per-partition decoupled-lookback chained scan + scatter. Reads
/// `keys_in`/`values_in`, writes to `keys_out`/`values_out`. The
/// dispatcher swaps the buffers between passes (ping-pong).
pub(crate) const ONESWEEP_SCATTER_WGSL: &str =
    include_str!("../../shaders/onesweep_scatter.wgsl");

/// Karras LBVH pass 1: write the N leaf nodes into nodes[N-1..2N-1)
/// using the Morton-permuted lookup `original_aabbs[sorted_indices[k]]`.
pub(crate) const KARRAS_LEAVES_WGSL: &str =
    include_str!("../../shaders/karras_leaves.wgsl");

/// Karras LBVH pass 2: parallel construction of the N-1 internal
/// nodes via Karras 2012's delta + range + split algorithm. Writes
/// node payloads + parent pointers; AABBs are filled by pass 3.
pub(crate) const KARRAS_INTERNAL_WGSL: &str =
    include_str!("../../shaders/karras_internal.wgsl");

/// Karras LBVH pass 3: bottom-up AABB propagation. Each leaf walks
/// up the parent chain; the second child to arrive at each internal
/// node merges both children's AABBs into the parent.
pub(crate) const KARRAS_AABB_WGSL: &str =
    include_str!("../../shaders/karras_aabb.wgsl");

/// TLAS pass 0: Morton encode every live chunk's centre against the
/// scene-wide bounds. Mirror of `morton.wgsl` but reads
/// `array<ChunkDescriptor>` so each chunk reduces to a single Morton
/// code (the TLAS leaf key).
pub(crate) const TLAS_MORTON_WGSL: &str =
    include_str!("../../shaders/tlas_morton.wgsl");

/// TLAS pass 2: write the N leaf nodes into the tail of `tlas_nodes`,
/// each encoded with `right_or_count = chunk_idx | BVH_LEAF_FLAG`
/// (`accel::tlas::encode_live`). Lays down live leaves only — the
/// dead-skip flag is the job of the eviction path.
pub(crate) const TLAS_LEAVES_WGSL: &str =
    include_str!("../../shaders/tlas_leaves.wgsl");

/// TLAS pass 3: parallel construction of N-1 internal nodes via
/// Karras 2012's delta + range + split algorithm. Mirror of
/// `karras_internal.wgsl` algorithmically — the only divergence is
/// the `parents[]` / `done[]` indexing convention (TLAS-specific:
/// leaves at `[0, N)`, internals at `[N, 2N - 1)`).
pub(crate) const TLAS_INTERNAL_WGSL: &str =
    include_str!("../../shaders/tlas_internal.wgsl");

/// TLAS pass 4: bottom-up AABB propagation, one tree level per
/// dispatch. Mirror of `karras_aabb.wgsl` algorithmically with the
/// TLAS-specific `done[]` indexing convention.
pub(crate) const TLAS_AABB_WGSL: &str =
    include_str!("../../shaders/tlas_aabb.wgsl");

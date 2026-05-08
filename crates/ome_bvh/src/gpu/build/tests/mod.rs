//! Golden CPU/GPU consistency tests for `Bvh::build_gpu` + the
//! refit-vs-CPU-ground-truth tests for `refit_gpu`.
//!
//! Build sizes cover the structural edge cases:
//!
//! - **N = 1**: leaves-only dispatch (no internals). Verifies the
//!   `n >= 2` orchestrator guard.
//! - **N = 2**: smallest non-trivial Karras tree (1 internal +
//!   2 leaves). Catches off-by-one in `range_and_split` for the
//!   lower bound — N = 8 + masks this.
//! - **N = 8**: balanced grid, sub-tile (one onesweep partition).
//! - **N = 100**: random AABBs, asymmetric split, sub-tile.
//! - **N = 1024**: balanced 32 × 32 grid, exactly one onesweep
//!   partition.
//! - **N = 65 000**: 22 onesweep partitions + ~16 levels of AABB
//!   propagation. Stress-tests the decoupled-lookback chained scan
//!   and the AABB iteration count, plus the buffer growth path.
//!
//! Refit sizes mirror the build sizes (minus the 65k stress test):
//! build → translate every AABB by a small delta → refit → readback.
//! Compared byte-exact against a CPU ground-truth that walks the
//! captured topology with the new AABBs.
//!
//! Each test uses a deterministic seed so failures are reproducible.

mod build;
mod helpers;
mod refit;
mod refit_helpers;

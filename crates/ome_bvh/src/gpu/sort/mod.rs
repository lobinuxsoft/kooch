//! Onesweep radix sort dispatch + pipelines.
//!
//! Hosts the WGSL pipelines for `init` (clear scratch buffers),
//! `histogram` (count digits across 4 passes in one dispatch),
//! `exclusive_scan` (prefix-sum the histogram per pass) and (in
//! round C) `scatter` (decoupled-lookback chained scan + scatter).
//!
//! [`SortPipelines`] owns the compiled pipelines and their bind group
//! layouts (created once, reused across builds). [`SortBuffers`] owns
//! the storage buffers (grow-on-demand, reused across builds).
//!
//! End-to-end sort orchestration lives in `Bvh::build_gpu` (subtask 4
//! of PR-3); this module exposes the per-pass primitives so each can
//! be unit-tested in isolation against the CPU reference.

mod buffers;
mod config;
mod dispatch;
mod pipelines;

#[cfg(test)]
mod testing;

/// Initial capacity for the keys / values buffers, in items.
/// Grows by `next_power_of_two` on demand.
const INITIAL_KEYS_CAPACITY: u64 = 1024;

/// Initial capacity for the partition descriptors, in partitions.
const INITIAL_PARTITIONS: u32 = 64;

pub use buffers::SortBuffers;
pub use config::{HistogramConfig, InitConfig, ScanConfig};
pub use dispatch::{
    dispatch_exclusive_scan, dispatch_histogram, dispatch_init, dispatch_scatter, dispatch_sort,
    dispatch_sort_into,
};
pub use pipelines::SortPipelines;

#[cfg(test)]
pub use testing::readback_histogram_for_test;

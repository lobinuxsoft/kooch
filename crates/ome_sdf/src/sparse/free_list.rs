//! Free-list bookkeeping for the subgrid pool.
//!
//! The free list is a flat `array<u32>` of length `max_subgrids`
//! paired with the `free_top` atomic counter in `counters_buffer`.
//! `free_list[free_top - 1]` is the next index a `pop` returns;
//! `push` writes at `free_list[free_top]` and increments. WGSL
//! pop / push helpers live in `shaders/sparse_freelist.wgsl`
//! (`SPARSE_FREELIST_WGSL`).
//!
//! # Init choice — `queue.write_buffer` over compute init
//!
//! Initialisation runs once per chunk creation, not in the per-frame
//! hot loop, so the GPU-driven constraint that drives the rest of
//! this crate does not bite here. `queue.write_buffer` ships the
//! 4 KiB identity permutation + 16 B counters in 8 lines of host
//! code; a compute init pass would add a shader file, a pipeline,
//! and a bind group for a one-shot dispatch that runs once per chunk
//! load. The simpler path wins. Re-evaluate only if profiling shows
//! `write_buffer` staging dominating chunk-load time.

use bytemuck::{Pod, Zeroable};

/// Counters mirror — must match the WGSL `SparseCounters` layout in
/// `sparse_freelist.wgsl` byte-for-byte (4 × `u32`, 16 B total).
///
/// `alloc_count_total` / `free_count_total` are the cumulative
/// successful pop / push counters (S8 metrics). Repurposed from the
/// original `_pad0` / `_pad1` slots so the buffer layout stays at
/// 16 B and existing freelist consumers (populate, free) need no
/// binding changes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub(super) struct CountersInit {
    pub free_top: u32,
    pub alloc_failed_count: u32,
    pub alloc_count_total: u32,
    pub free_count_total: u32,
}

/// Seed `free_list_buffer` with the identity permutation
/// `[0, 1, …, max_subgrids - 1]` and `counters_buffer` with
/// `free_top = max_subgrids`, all other counters zero.
///
/// After this call any consumer of `SPARSE_FREELIST_WGSL` can pop up
/// to `max_subgrids` indices before the pool is exhausted.
pub(super) fn init(
    queue: &wgpu::Queue,
    free_list_buffer: &wgpu::Buffer,
    counters_buffer: &wgpu::Buffer,
    max_subgrids: u32,
) {
    let indices: Vec<u32> = (0..max_subgrids).collect();
    queue.write_buffer(free_list_buffer, 0, bytemuck::cast_slice(&indices));

    let counters = CountersInit {
        free_top: max_subgrids,
        alloc_failed_count: 0,
        alloc_count_total: 0,
        free_count_total: 0,
    };
    queue.write_buffer(counters_buffer, 0, bytemuck::bytes_of(&counters));
}

#[cfg(test)]
mod tests;

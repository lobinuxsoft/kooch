//! Subgrid pool free list: `free_list[free_top - 1]` pops next. Seeded with `queue.write_buffer`,
//! since a compute pass for a 4 KiB once-per-load write is not worth a shader.

use bytemuck::{Pod, Zeroable};

/// Mirror of WGSL `SparseCounters`, 4 × `u32` = 16 B. The totals took over the old padding slots,
/// so no consumer changed bindings.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub(super) struct CountersInit {
    pub free_top: u32,
    pub alloc_failed_count: u32,
    pub alloc_count_total: u32,
    pub free_count_total: u32,
}

/// Seeds the identity permutation and `free_top = max_subgrids`, so up to `max_subgrids` pops
/// succeed.
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

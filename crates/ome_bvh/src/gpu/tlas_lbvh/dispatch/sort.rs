//! TLAS pass 1 — onesweep radix sort of the per-chunk Morton codes.

use super::super::TlasGpuBuilder;
use crate::gpu::sort::dispatch_sort_into;

impl TlasGpuBuilder {
    /// Pass 1 of the TLAS rebuild: sort `tlas_mortons` ascending and
    /// emit the matching permutation into `tlas_sorted_indices`. Reuses
    /// the BLAS [`crate::gpu::sort`] onesweep pipelines as-is — only
    /// the buffer plumbing differs.
    ///
    /// Onesweep keeps its keys / values inside `self.sort_buffers`. We
    /// stage the inputs by GPU-to-GPU copy (no CPU readback) and copy
    /// the sorted outputs back to the caller-supplied scratch:
    ///
    /// 1. Identity payload `[0, n)` → `sort_buffers.values_a`
    ///    (`queue.write_buffer`, ≤ 4 KiB at the default 1024-chunk cap).
    /// 2. `tlas_mortons` → `sort_buffers.keys_a` (encoder copy).
    /// 3. [`dispatch_sort_into`] runs init + 4 radix passes; even pass
    ///    count returns the result to `_a`.
    /// 4. `sort_buffers.keys_a` → `tlas_mortons` (sorted in place).
    /// 5. `sort_buffers.values_a` → `tlas_sorted_indices`.
    ///
    /// Caller invariants:
    /// - `tlas_mortons` was written by [`Self::dispatch_morton`] in
    ///   the same encoder (or pre-populated with the equivalent keys).
    /// - `self.ensure_capacity(device, n as u64)` was called once after
    ///   construction to grow the onesweep scratch to fit the chunk pool.
    /// - `n == 0` is a no-op.
    pub fn dispatch_sort(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        tlas_mortons: &wgpu::Buffer,
        tlas_sorted_indices: &wgpu::Buffer,
        n: u32,
    ) {
        if n == 0 {
            return;
        }

        let bytes = (n as u64) * 4;

        // 1. Identity payload — onesweep treats values_a as the input
        // permutation, so a `[0, n)` fill before sort means values_a
        // ends up holding `original_index_at_sorted_position[k]`.
        let identity: Vec<u32> = (0..n).collect();
        queue.write_buffer(
            &self.sort_buffers.values_a,
            0,
            bytemuck::cast_slice(&identity),
        );

        // 2. Stage the keys: tlas_mortons → sort_buffers.keys_a.
        encoder.copy_buffer_to_buffer(tlas_mortons, 0, &self.sort_buffers.keys_a, 0, bytes);

        // 3. Run the onesweep: init + histogram + 4 scans + 4 scatters.
        // RADIX_PASSES is 4 (even), so the final result lands back in
        // the `_a` slot. If anyone ever bumps RADIX_PASSES to an odd
        // count without updating this consumer, the read-back below
        // pulls stale bytes from the wrong slot. Hardening tracked in
        // issue #373 (compile-time gate in sort_types.rs).
        dispatch_sort_into(
            device,
            queue,
            encoder,
            &self.sort_pipelines,
            &self.sort_buffers,
            n,
        );

        // 4. Copy the sorted keys back to the caller-visible buffer.
        encoder.copy_buffer_to_buffer(&self.sort_buffers.keys_a, 0, tlas_mortons, 0, bytes);

        // 5. Copy the permutation to the caller-visible buffer.
        encoder.copy_buffer_to_buffer(
            &self.sort_buffers.values_a,
            0,
            tlas_sorted_indices,
            0,
            bytes,
        );
    }
}

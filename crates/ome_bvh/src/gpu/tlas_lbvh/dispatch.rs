//! Per-pass dispatch helpers for the TLAS Karras LBVH GPU pipeline.
//!
//! Each function records its compute pass into the caller-supplied
//! encoder; the orchestration loop in
//! [`super::TlasGpuBuilder::dispatch_rebuild`] (lands in commit 7)
//! chains them together so morton + sort + leaves + internal + aabb
//! share a single submission and a single CPU side-effect window.

use super::{KarrasConfig, TlasGpuBuilder};
use crate::gpu::sort::dispatch_sort_into;
use crate::gpu::types::GpuSceneBounds;

/// Workgroup size for the Morton pass — matches the BLAS
/// `morton.wgsl` workgroup size so the encoding stays byte-identical
/// and avoids per-vendor tuning.
const MORTON_WORKGROUP_SIZE: u32 = 256;

impl TlasGpuBuilder {
    /// Pass 0 of the TLAS rebuild: write per-chunk Morton codes into
    /// `tlas_mortons` for the subsequent onesweep sort. Safe for any
    /// `n` — `n == 0` is a no-op (early-out from the orchestrator).
    pub fn dispatch_morton(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        chunk_descriptors: &wgpu::Buffer,
        tlas_mortons: &wgpu::Buffer,
        scene: GpuSceneBounds,
        n: u32,
    ) {
        if n == 0 {
            return;
        }

        queue.write_buffer(&self.scene_bounds_buffer, 0, bytemuck::bytes_of(&scene));
        let cfg = KarrasConfig {
            n,
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
        };
        queue.write_buffer(&self.config_buffer, 0, bytemuck::bytes_of(&cfg));

        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ome_bvh::tlas_morton_bg"),
            layout: &self.morton_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: chunk_descriptors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.scene_bounds_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tlas_mortons.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.config_buffer.as_entire_binding(),
                },
            ],
        });
        let workgroups = n.div_ceil(MORTON_WORKGROUP_SIZE);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ome_bvh::tlas_morton_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.morton_pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(workgroups.max(1), 1, 1);
    }

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
        encoder.copy_buffer_to_buffer(
            tlas_mortons,
            0,
            &self.sort_buffers.keys_a,
            0,
            bytes,
        );

        // 3. Run the onesweep: init + histogram + 4 scans + 4 scatters.
        // RADIX_PASSES is 4 (even), so the final result lands back in
        // the `_a` slot.
        dispatch_sort_into(
            device,
            queue,
            encoder,
            &self.sort_pipelines,
            &self.sort_buffers,
            n,
        );

        // 4. Copy the sorted keys back to the caller-visible buffer.
        encoder.copy_buffer_to_buffer(
            &self.sort_buffers.keys_a,
            0,
            tlas_mortons,
            0,
            bytes,
        );

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

//! TLAS pass 2 — write the N leaf nodes into the tail of `tlas_nodes`.

use super::super::TlasGpuBuilder;
use crate::gpu::karras_common::KARRAS_WORKGROUP_SIZE;

impl TlasGpuBuilder {
    /// Pass 2 of the TLAS rebuild: write the N leaf nodes into the tail
    /// of `tlas_nodes` at indices `[N-1, 2N-1)` (or `[0, 1)` when
    /// `n == 1`), each encoded with `right_or_count = chunk_idx |
    /// BVH_LEAF_FLAG`. Sets `tlas_done[k] = 1` for k ∈ [0, N) so the
    /// upcoming AABB propagation pass sees finalised leaves.
    ///
    /// Caller invariants:
    /// - `tlas_sorted_indices` was populated by [`Self::dispatch_sort`]
    ///   in the same encoder.
    /// - `tlas_nodes` is at least `(2N - 1) * size_of::<BvhNode>` bytes.
    /// - `tlas_done` is at least `2N * 4` bytes (commit 6a).
    /// - `n == 0` is a no-op.
    pub fn dispatch_leaves(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        tlas_nodes: &wgpu::Buffer,
        tlas_sorted_indices: &wgpu::Buffer,
        chunk_descriptors: &wgpu::Buffer,
        tlas_done: &wgpu::Buffer,
        live_chunk_indices: &wgpu::Buffer,
        n: u32,
    ) {
        if n == 0 {
            return;
        }

        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ome_bvh::tlas_leaves_bg"),
            layout: &self.leaves_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: tlas_nodes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tlas_sorted_indices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: chunk_descriptors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: tlas_done.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: live_chunk_indices.as_entire_binding(),
                },
            ],
        });
        let workgroups = n.div_ceil(KARRAS_WORKGROUP_SIZE);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ome_bvh::tlas_leaves_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.leaves_pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(workgroups.max(1), 1, 1);
    }
}

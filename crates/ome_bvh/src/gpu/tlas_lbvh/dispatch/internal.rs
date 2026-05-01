//! TLAS pass 3 — parallel construction of the N-1 Karras internals.

use super::super::TlasGpuBuilder;
use crate::gpu::karras_common::KARRAS_WORKGROUP_SIZE;

impl TlasGpuBuilder {
    /// Pass 3 of the TLAS rebuild: parallel construction of the N-1
    /// internal Karras nodes. Each thread resolves its node's range +
    /// split via the canonical Karras 2012 algorithm and writes
    /// `tlas_nodes[i]` for i ∈ [0, N-1), recording parent pointers
    /// using the TLAS-specific `role_idx` convention (leaves at
    /// `[0, N)`, internals at `[N, 2N - 1)` of `parents` / `done`).
    ///
    /// Caller invariants:
    /// - `tlas_mortons` holds the **sorted ascending** Morton codes
    ///   (output of [`Self::dispatch_sort`] in the same encoder).
    /// - `tlas_parents` and `tlas_done` are sized at `2 × max_chunks`
    ///   `u32` slots (commit 6a).
    /// - `n >= 2`. `n in {0, 1}` has no internals to build — caller
    ///   must skip this dispatch (a 1-leaf TLAS is also a 1-node tree
    ///   where the leaf IS the root).
    pub fn dispatch_internal(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        tlas_nodes: &wgpu::Buffer,
        tlas_mortons: &wgpu::Buffer,
        tlas_parents: &wgpu::Buffer,
        tlas_done: &wgpu::Buffer,
        n: u32,
    ) {
        debug_assert!(
            n >= 2,
            "dispatch_internal requires n >= 2 — orchestrator must skip \
             this pass for n in {{0, 1}}",
        );

        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ome_bvh::tlas_internal_bg"),
            layout: &self.internal_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: tlas_nodes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tlas_mortons.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tlas_parents.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: tlas_done.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.config_buffer.as_entire_binding(),
                },
            ],
        });
        let workgroups = (n - 1).div_ceil(KARRAS_WORKGROUP_SIZE);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ome_bvh::tlas_internal_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.internal_pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(workgroups.max(1), 1, 1);
    }
}

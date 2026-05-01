//! TLAS pass 4 — bottom-up AABB propagation, host-looped one tree
//! level per dispatch.

use super::super::TlasGpuBuilder;
use crate::gpu::karras_common::{KARRAS_WORKGROUP_SIZE, aabb_iterations};

impl TlasGpuBuilder {
    /// Pass 4 of the TLAS rebuild: bottom-up AABB propagation. The
    /// shader is single-step (one tree level per dispatch); we loop
    /// `aabb_iterations(n) = 2 * log_n + 4` times to converge every
    /// internal under any topology Karras can produce. The dispatch
    /// boundary doubles as the cross-workgroup memory barrier WGSL
    /// otherwise lacks.
    ///
    /// Caller invariants:
    /// - Leaves AABBs are already populated (output of
    ///   [`Self::dispatch_leaves`]) and `tlas_done[k] = 1` for
    ///   k ∈ [0, N).
    /// - Internal nodes' children pointers are populated (output of
    ///   [`Self::dispatch_internal`]) and `tlas_done[N + i] = 0` for
    ///   i ∈ [0, N - 1).
    /// - `n >= 2`. `n in {0, 1}` has no internals to propagate —
    ///   caller must skip this pass.
    pub fn dispatch_aabb(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        tlas_nodes: &wgpu::Buffer,
        tlas_parents: &wgpu::Buffer,
        tlas_done: &wgpu::Buffer,
        n: u32,
    ) {
        debug_assert!(
            n >= 2,
            "dispatch_aabb requires n >= 2 — orchestrator must skip \
             this pass for n in {{0, 1}}",
        );

        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ome_bvh::tlas_aabb_bg"),
            layout: &self.aabb_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: tlas_nodes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tlas_parents.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tlas_done.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.config_buffer.as_entire_binding(),
                },
            ],
        });
        // Internals dispatch over (n - 1) threads. Workgroup count
        // matches the internal pass for symmetry.
        let workgroups = (n - 1).div_ceil(KARRAS_WORKGROUP_SIZE);
        let iterations = aabb_iterations(n);

        for iter in 0..iterations {
            let label = format!("ome_bvh::tlas_aabb_pass_{iter}");
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(&label),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.aabb_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(workgroups.max(1), 1, 1);
        }
    }
}

//! TLAS pass 0 — Morton encode every live chunk's centre.

use super::super::{KarrasConfig, TlasGpuBuilder};
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
        live_chunk_indices: &wgpu::Buffer,
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
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: live_chunk_indices.as_entire_binding(),
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
}

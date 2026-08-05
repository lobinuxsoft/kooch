use crate::meshlet::cull::CullParams;
use crate::meshlet::gpu_meshlet::GpuMeshletMesh;

use super::super::MeshletCull;
use super::super::pipelines::MeshletCullPipelines;
use super::super::types::HiZTestParams;

impl MeshletCull {
    /// Dispatches the cull pass for `mesh` against `params`. Resets
    /// `visible_count` to zero before dispatch so each frame starts
    /// from a clean slate.
    ///
    /// The caller must keep `mesh` alive for the duration of the
    /// encoder submission — bind groups borrow its descriptor buffer.
    pub fn dispatch(
        &self,
        pipelines: &MeshletCullPipelines,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        mesh: &GpuMeshletMesh,
        params: &CullParams,
    ) {
        debug_assert!(
            mesh.meshlet_count <= self.capacity,
            "meshlet count {} exceeds dispatcher capacity {}",
            mesh.meshlet_count,
            self.capacity,
        );

        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(params));
        encoder.clear_buffer(&self.visible_count, 0, None);

        let cull_bg = self.build_cull_bg(pipelines, device, mesh);

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.pipeline);
            pass.set_bind_group(0, &cull_bg, &[]);
            let workgroups = mesh.meshlet_count.div_ceil(64);
            pass.dispatch_workgroups(workgroups.max(1), 1, 1);
        }

        self.mirror_count_to_indirect_args(encoder);
    }

    /// Same as [`Self::dispatch`] but runs the Hi-Z-aware cull entry
    /// point. `hi_z_view` must reference the multi-mip pyramid view
    /// produced by [`crate::hi_z::HiZ::full_view`].
    pub fn dispatch_with_hi_z(
        &self,
        pipelines: &MeshletCullPipelines,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        mesh: &GpuMeshletMesh,
        params: &CullParams,
        hi_z_params: &HiZTestParams,
        hi_z_view: &wgpu::TextureView,
    ) {
        debug_assert!(
            mesh.meshlet_count <= self.capacity,
            "meshlet count {} exceeds dispatcher capacity {}",
            mesh.meshlet_count,
            self.capacity,
        );

        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(params));
        queue.write_buffer(&self.hi_z_params_buffer, 0, bytemuck::bytes_of(hi_z_params));
        encoder.clear_buffer(&self.visible_count, 0, None);

        let cull_bg = self.build_cull_bg(pipelines, device, mesh);
        let hi_z_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_hi_z_bg"),
            layout: &pipelines.hi_z_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.hi_z_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(hi_z_view),
                },
            ],
        });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_hi_z_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.pipeline_hi_z);
            pass.set_bind_group(0, &cull_bg, &[]);
            pass.set_bind_group(1, &hi_z_bg, &[]);
            let workgroups = mesh.meshlet_count.div_ceil(64);
            pass.dispatch_workgroups(workgroups.max(1), 1, 1);
        }

        self.mirror_count_to_indirect_args(encoder);
    }
}

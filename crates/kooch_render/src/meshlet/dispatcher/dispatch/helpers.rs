use crate::meshlet::gpu_meshlet::GpuMeshletMesh;

use super::super::MeshletCull;
use super::super::pipelines::MeshletCullPipelines;

impl MeshletCull {
    pub(super) fn build_cull_bg(
        &self,
        pipelines: &MeshletCullPipelines,
        device: &wgpu::Device,
        mesh: &GpuMeshletMesh,
        params: wgpu::BufferBinding<'_>,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_bg_dispatch"),
            layout: &pipelines.cull_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(params),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: mesh.descriptors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.visible_meshlets.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.visible_count.as_entire_binding(),
                },
            ],
        })
    }

    /// Mirror the atomic visible counter into the indirect args'
    /// `instance_count` slot. Offset 4 inside DrawIndirectArgs:
    ///   [0..4)   vertex_count    (constant, set at construction)
    ///   [4..8)   instance_count  (this copy)
    ///   [8..16)  first_vertex / first_instance (zero, immutable)
    pub(super) fn mirror_count_to_indirect_args(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_buffer_to_buffer(&self.visible_count, 0, &self.indirect_args, 4, 4);
    }
}

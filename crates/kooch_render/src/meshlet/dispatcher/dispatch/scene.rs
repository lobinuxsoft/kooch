use crate::meshlet::cull::CullParams;
use crate::meshlet::gpu_meshlet::GpuMeshletMesh;
use crate::meshlet::scene::{MeshletScene, SceneCullParams};

use super::super::MeshletCull;
use super::super::pipelines::MeshletCullPipelines;

impl MeshletCull {
    /// Scene-wide cull against a SINGLE [`GpuMeshletMesh`]
    /// (Phase 1.E.1). Drives `cs_cull_scene` directly without going
    /// through the [`GpuGlobalMeshPool`]; production code uses
    /// [`Self::dispatch_scene_pool`] instead, but tests retain this
    /// path to validate the cull shader at low level without
    /// constructing a pool. New callers should prefer the pool path.
    ///
    /// # Capacity
    ///
    /// `MeshletCull::capacity` must cover the worst-case sum of visible
    /// meshlets (`instance_count * meshlets_per_mesh`). The dispatcher
    /// debug-asserts this on entry.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_scene(
        &self,
        pipelines: &MeshletCullPipelines,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        mesh: &GpuMeshletMesh,
        scene: &MeshletScene,
        cull_params: &CullParams,
        scene_params: &SceneCullParams,
    ) {
        let total_threads = scene_params.instance_count * scene_params.meshlets_per_mesh;
        debug_assert!(
            total_threads <= self.capacity,
            "scene total (instances × meshlets/mesh) {} exceeds dispatcher capacity {}",
            total_threads,
            self.capacity,
        );

        let params_binding = self.stage_params(queue, cull_params);
        queue.write_buffer(
            &self.scene_params_buffer,
            0,
            bytemuck::bytes_of(scene_params),
        );
        encoder.clear_buffer(&self.visible_count, 0, None);

        let cull_bg = self.build_cull_bg(pipelines, device, mesh, params_binding);
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_scene_bg"),
            layout: &pipelines.scene_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: scene.instance_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.scene_params_buffer.as_entire_binding(),
                },
            ],
        });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meshlet_cull_scene_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.pipeline_scene);
            pass.set_bind_group(0, &cull_bg, &[]);
            pass.set_bind_group(2, &scene_bg, &[]);
            let workgroups = total_threads.div_ceil(64).max(1);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        self.mirror_count_to_indirect_args(encoder);
    }
}

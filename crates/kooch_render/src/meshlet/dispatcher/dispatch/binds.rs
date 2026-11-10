//! The scene-wide cull's bind groups, kept between frames (#1869). Every buffer they bind lives
//! until a pool or a capacity grows, and the params ring is reached by a dynamic offset, so a frame
//! with nothing resized builds none.

use std::sync::{Arc, PoisonError};

use crate::meshlet::pool::GpuGlobalMeshPool;
use crate::meshlet::scene::MeshletScene;

use super::super::MeshletCull;
use super::super::pipelines::MeshletCullPipelines;

/// Groups 0–4 of the scene-wide culls; group 3 is `group_err` for the atomic cull and `chunked`
/// for the two-level one.
pub(crate) struct CullBinds {
    /// What the groups were built over: any of them replaced means rebuilt.
    bound: Vec<wgpu::Buffer>,
    pub cull: wgpu::BindGroup,
    pub pool: wgpu::BindGroup,
    pub scene: wgpu::BindGroup,
    pub group_err: wgpu::BindGroup,
    pub chunked: wgpu::BindGroup,
    pub debug: wgpu::BindGroup,
}

impl MeshletCull {
    /// The bind groups over `pool` and `scene`, built only when a buffer they bind was replaced.
    pub(super) fn binds(
        &self,
        pipelines: &MeshletCullPipelines,
        device: &wgpu::Device,
        pool: &GpuGlobalMeshPool,
        scene: &MeshletScene,
    ) -> Arc<CullBinds> {
        let bound = [
            &pool.meshlets,
            &pool.mesh_descriptors,
            &pool.mesh_bounds,
            scene.instance_buffer(),
            &self.params_buffer,
            &self.scene_params_buffer,
            &self.visible_meshlets,
            &self.visible_count,
            &self.group_max_err,
            &self.chunks,
            &self.reject_reasons,
            &self.stage_counters,
        ];
        let mut kept = self.binds.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(binds) = kept.as_ref()
            && binds.bound.iter().eq(bound.iter().copied())
        {
            return binds.clone();
        }
        let binds = Arc::new(build(self, pipelines, device, pool, scene, &bound));
        *kept = Some(binds.clone());
        binds
    }

    /// One slot of the params ring at offset 0, for `cull_bgl`, which moves it by a dynamic offset.
    pub(super) fn params_window(&self) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.params_buffer,
            offset: 0,
            size: std::num::NonZeroU64::new(
                std::mem::size_of::<crate::meshlet::cull::CullParams>() as u64,
            ),
        })
    }
}

fn build(
    cull: &MeshletCull,
    pipelines: &MeshletCullPipelines,
    device: &wgpu::Device,
    pool: &GpuGlobalMeshPool,
    scene: &MeshletScene,
    bound: &[&wgpu::Buffer],
) -> CullBinds {
    let group = |label, layout, resources: &[wgpu::BindingResource<'_>]| {
        let entries: Vec<wgpu::BindGroupEntry<'_>> = resources
            .iter()
            .enumerate()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: resource.clone(),
            })
            .collect();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &entries,
        })
    };
    CullBinds {
        bound: bound.iter().map(|&buffer| buffer.clone()).collect(),
        cull: group(
            "meshlet_cull_scene_cull_bg",
            &pipelines.cull_bgl,
            &[
                cull.params_window(),
                pool.meshlets.as_entire_binding(),
                cull.visible_meshlets.as_entire_binding(),
                cull.visible_count.as_entire_binding(),
            ],
        ),
        pool: group(
            "meshlet_cull_scene_pool_bg",
            &pipelines.pool_bgl,
            &[
                pool.mesh_descriptors.as_entire_binding(),
                pool.meshlets.as_entire_binding(),
            ],
        ),
        scene: group(
            "meshlet_cull_scene_scene_bg",
            &pipelines.scene_bgl,
            &[
                scene.instance_buffer().as_entire_binding(),
                cull.scene_params_buffer.as_entire_binding(),
            ],
        ),
        group_err: group(
            "meshlet_group_err_bg",
            &pipelines.group_err_bgl,
            &[cull.group_max_err.as_entire_binding()],
        ),
        chunked: group(
            "meshlet_cull_chunked_bg",
            &pipelines.chunked_bgl,
            &[
                cull.group_max_err.as_entire_binding(),
                pool.mesh_bounds.as_entire_binding(),
                cull.chunks.as_entire_binding(),
            ],
        ),
        debug: group(
            "meshlet_cull_debug_bg",
            &pipelines.debug_bgl,
            &[
                cull.reject_reasons.as_entire_binding(),
                cull.stage_counters.as_entire_binding(),
            ],
        ),
    }
}

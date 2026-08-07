//! The shadow pass as one object: place, cull, draw.
//!
//! The atlas and the rasteriser are always used together and neither is
//! useful alone, so the render stage holds one field rather than two and
//! the ordering between them stays in this file. It is also the only
//! place that knows a shadow pass has to be recorded **before** the
//! shading that samples it.

use glam::Vec3;

use crate::meshlet::{GpuGlobalMeshPool, MeshletCullPipelines, MeshletScene};
use crate::view_camera::ViewCamera;

use super::atlas::ShadowAtlas;
use super::cascades::{CASCADE_BLEND_FRACTION, CASCADE_COUNT, Cascade, build_cascades};
use super::raster::ShadowRasterizer;

/// How far behind a cascade's sphere the light's eye sits, in radii.
///
/// This is what makes an occluder *outside* the view frustum cast into
/// it: a wall behind the camera shadows the floor in front of it only if
/// the light's near plane is behind the wall. Larger is safer and
/// spreads the same depth buffer over more distance, so it trades a
/// missing shadow for a less precise one — and a missing shadow is the
/// one people report.
const DEPTH_EXTENT_SCALE: f32 = 4.0;

/// The atlas, the pipeline, and the ordering between them.
pub struct ShadowPass {
    atlas: ShadowAtlas,
    rasterizer: ShadowRasterizer,
}

/// A frame's placed cascades: the matrices the pass draws with, and the
/// records the shading model samples with.
pub struct PreparedShadows {
    cascades: [Cascade; CASCADE_COUNT],
    /// What goes in the frame UBO. Handed to
    /// [`kooch_lighting::GpuLights::update`].
    pub frame: kooch_lighting::FrameShadows,
}

impl ShadowPass {
    pub fn new(
        device: &wgpu::Device,
        meshlet_bgl: &wgpu::BindGroupLayout,
        cascade_size: u32,
        instance_capacity: u32,
        max_triangles_per_meshlet: u32,
    ) -> Self {
        Self {
            atlas: ShadowAtlas::new(
                device,
                cascade_size,
                instance_capacity,
                max_triangles_per_meshlet,
            ),
            rasterizer: ShadowRasterizer::new(device, meshlet_bgl),
        }
    }

    /// The atlas texture, for binding into Inti's group.
    pub fn atlas_view(&self) -> &wgpu::TextureView {
        self.atlas.view()
    }

    pub fn atlas_bytes(&self) -> u64 {
        self.atlas.byte_size()
    }

    /// Places this frame's cascades and sizes the culls for the scene.
    ///
    /// `max_distance` cuts the camera's frustum short before the
    /// cascades are fitted to it — see [`ViewCamera::projection_to`].
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        camera: &ViewCamera,
        aspect: f32,
        sun_direction: Vec3,
        max_distance: f32,
        meshlet_capacity: u32,
        group_capacity: u32,
    ) -> PreparedShadows {
        self.atlas
            .ensure_capacity(device, meshlet_capacity, group_capacity);

        let far = camera.far.min(max_distance.max(camera.near + 1e-3));
        let shadow_view_proj = camera.projection_to(aspect, far) * camera.view();
        let cascades = build_cascades(
            shadow_view_proj,
            sun_direction,
            camera.near,
            far,
            self.atlas.cascade_size(),
            DEPTH_EXTENT_SCALE,
        );

        PreparedShadows {
            frame: kooch_lighting::FrameShadows {
                camera_forward: camera.forward(),
                cascades: self.atlas.gpu_cascades(&cascades),
                blend: CASCADE_BLEND_FRACTION,
            },
            cascades,
        }
    }

    /// Records the cull and depth passes for every cascade.
    ///
    /// Must be recorded into the frame's encoder **before** any pass
    /// that samples the atlas. Nothing enforces that — an encoder is a
    /// list and the ordering is the caller's, which is why the only
    /// caller is the render stage's frame orchestrator.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        prepared: &PreparedShadows,
        cull_pipelines: &MeshletCullPipelines,
        pool: &GpuGlobalMeshPool,
        scene: &MeshletScene,
        meshlet_bg: &wgpu::BindGroup,
        instance_count: u32,
        max_meshlets_per_mesh: u32,
        lod_target: f32,
    ) {
        self.rasterizer.render(
            device,
            queue,
            encoder,
            &self.atlas,
            &prepared.cascades,
            cull_pipelines,
            pool,
            scene,
            meshlet_bg,
            instance_count,
            max_meshlets_per_mesh,
            lod_target,
        );
    }
}

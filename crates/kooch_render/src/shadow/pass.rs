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
use super::cascades::{build_cascades, Cascade, CASCADE_BLEND_FRACTION, CASCADE_COUNT};
use super::cube::{PointShadowCubes, DEFAULT_CUBE_SIZE};
use super::point::PointShadowDraw;
use super::raster::ShadowRasterizer;

/// The atlas, the point lights' cubes, the pipeline, and the ordering
/// between them.
pub struct ShadowPass {
    atlas: ShadowAtlas,
    /// Separate texture from the atlas, at its own size — see
    /// [`PointShadowCubes`].
    cubes: PointShadowCubes,
    rasterizer: ShadowRasterizer,
}

/// A frame's placed cascades: the matrices the pass draws with, and the
/// records the shading model samples with.
pub struct PreparedShadows {
    cascades: [Cascade; CASCADE_COUNT],
    /// One per shadow-casting spot light this frame, already fitted
    /// (#777). Empty is the common case and costs nothing.
    spots: Vec<super::SpotShadowDraw>,
    /// One per shadow-casting point light this frame (#778). Six draws
    /// each, so an empty list is worth having.
    pub points: Vec<PointShadowDraw>,
    /// What goes in the frame UBO. Handed to
    /// [`kooch_lighting::GpuLights::update`].
    pub frame: kooch_lighting::FrameShadows,
}

impl ShadowPass {
    pub fn new(
        device: &wgpu::Device,
        meshlet_bgl: &wgpu::BindGroupLayout,
        cascade_size: u32,
        // Cube maps to allocate — the VRAM this pass costs beyond the
        // atlas, at 6 MiB each (#849).
        point_budget: u32,
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
            cubes: PointShadowCubes::new(
                device,
                DEFAULT_CUBE_SIZE,
                point_budget,
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

    /// The atlas texture itself, for reading back.
    ///
    /// Exists so a test can look at what the pass actually drew. "The
    /// shadow is wrong" splits into "the map is wrong" and "the
    /// sampling is wrong", and those have nothing in common but the
    /// symptom.
    pub fn atlas_texture(&self) -> &wgpu::Texture {
        self.atlas.texture()
    }

    pub fn atlas_bytes(&self) -> u64 {
        self.atlas.byte_size() + self.cubes.byte_size()
    }

    /// The point lights' cube array, for binding into Inti's group.
    pub fn cubes_view(&self) -> &wgpu::TextureView {
        self.cubes.view()
    }

    /// The cube array itself, for reading back in a test — the same
    /// reason `atlas_texture` exists.
    pub fn cubes_texture(&self) -> &wgpu::Texture {
        self.cubes.texture()
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
        cascades_enabled: bool,
        max_distance: f32,
        first_cascade_distance: f32,
        sun_softness: f32,
        spots: &[kooch_lighting::SpotShadowSource],
        points: &[kooch_lighting::PointShadowSource],
        meshlet_capacity: u32,
        group_capacity: u32,
    ) -> PreparedShadows {
        self.atlas
            .ensure_capacity(device, meshlet_capacity, group_capacity);
        self.cubes
            .ensure_capacity(device, meshlet_capacity, group_capacity);

        let far = camera.far.min(max_distance.max(camera.near + 1e-3));
        let shadow_view_proj = camera.projection_to(aspect, far) * camera.view();
        let cascades = build_cascades(
            shadow_view_proj,
            sun_direction,
            camera.near,
            far,
            first_cascade_distance.clamp(camera.near + 1e-3, far),
            self.atlas.cascade_size(),
            self.rasterizer.near_extension_scale(),
        );

        let texels = self.atlas.cascade_size();
        let draws: Vec<super::SpotShadowDraw> = spots
            .iter()
            .enumerate()
            .map(|(slot, source)| super::SpotShadowDraw {
                record: super::spot_shadow(source, ShadowAtlas::spot_layer(slot), texels),
                eye: source.position,
            })
            .collect();

        let mut spot_records =
            [kooch_lighting::GpuCascade::default(); kooch_lighting::MAX_SPOT_SHADOWS];
        for (slot, draw) in draws.iter().enumerate() {
            spot_records[slot] = draw.record;
        }

        let cube_size = self.cubes.size();
        let point_draws: Vec<PointShadowDraw> = points.iter().map(PointShadowDraw::new).collect();
        let mut point_records =
            [kooch_lighting::GpuPointShadow::default(); kooch_lighting::MAX_POINT_SHADOWS];
        let mut point_entities =
            [kooch_ecs::entity::Entity::INVALID; kooch_lighting::MAX_POINT_SHADOWS];
        for (slot, source) in points.iter().enumerate() {
            point_records[slot] = super::point_shadow(source, cube_size);
            point_entities[slot] = source.entity;
        }

        PreparedShadows {
            frame: kooch_lighting::FrameShadows {
                camera_forward: camera.forward(),
                cascades: self.atlas.gpu_cascades(&cascades),
                blend: CASCADE_BLEND_FRACTION,
                sun_softness,
                cascades_enabled,
                spot_shadows: spot_records,
                spot_shadow_count: draws.len() as u32,
                point_shadows: point_records,
                point_shadow_count: point_draws.len() as u32,
                point_entities,
            },
            cascades,
            spots: draws,
            points: point_draws,
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
        // Which point-light cubes actually need redrawing this frame,
        // with the slot each one occupies. Empty is the good case: a
        // static lamp in a static room draws its six faces once.
        redraw: &[(usize, PointShadowDraw)],
        cull_pipelines: &MeshletCullPipelines,
        pool: &GpuGlobalMeshPool,
        scene: &MeshletScene,
        meshlet_bg: &wgpu::BindGroup,
        instance_count: u32,
        max_meshlets_per_mesh: u32,
        lod_target: f32,
    ) {
        // 🔴 Skipped outright when the cascades have no reader. The flag
        // already existed and already reached the shading — `inti_shadow`
        // returns before touching a cascade layer when the virtual pages
        // are on — but nothing here consulted it, so four culls and four
        // depth passes ran every frame to fill layers nobody sampled.
        //
        // The call below is the ONLY thing gated. The spots and the
        // point cubes that follow share this atlas and this rasteriser
        // and have no page raster yet, so the allocation stays and so do
        // their draws.
        if prepared.frame.cascades_enabled {
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
        self.rasterizer.render_points(
            device,
            queue,
            encoder,
            &self.cubes,
            redraw,
            cull_pipelines,
            pool,
            scene,
            meshlet_bg,
            instance_count,
            max_meshlets_per_mesh,
            lod_target,
        );
        self.rasterizer.render_spots(
            device,
            queue,
            encoder,
            &self.atlas,
            &prepared.spots,
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

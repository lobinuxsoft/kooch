//! Wiring the shadow pass into a frame (#476).
//!
//! Two steps, in this order and for a reason:
//!
//! 1. [`MeshletRenderStage::prepare_shadows`] runs **before** the
//!    frame's encoder exists, because it can allocate — the atlas on the
//!    first sunlit frame, the cascade culls whenever the scene grows —
//!    and a buffer replaced after a pass references it is a use of the
//!    old one.
//! 2. [`MeshletRenderStage::record_shadows`] runs **first inside** the
//!    encoder, because everything that shades reads the atlas it fills.

use kooch_core::resource::Resources;

use crate::meshlet::extract_frustum_planes;
use crate::shadow::{PreparedShadows, ShadowPass, ShadowSettings};
use crate::view_camera::ViewCamera;

use super::super::MeshletRenderStage;

impl MeshletRenderStage {
    /// Allocates the atlas if this frame needs one, places the cascades
    /// and sizes the culls.
    ///
    /// `None` when nothing casts: no directional light with
    /// `cast_shadows`, or the author turned shadows off. The caller
    /// passes that straight through to `GpuLights::update`, which leaves
    /// the dummy atlas bound and the sampling switched off.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::meshlet::render_stage) fn prepare_shadows(
        &mut self,
        device: &wgpu::Device,
        resources: &Resources,
        camera: &ViewCamera,
        aspect: f32,
        meshlet_capacity: u32,
        group_capacity: u32,
    ) -> Option<PreparedShadows> {
        let settings = resources
            .get::<ShadowSettings>()
            .copied()
            .unwrap_or_default();
        let sun = kooch_lighting::shadow_casting_sun(resources);
        // Spot lights keep the array alive on their own (#777): a scene
        // lit by a torch and no sun still casts, and releasing the
        // texture because nothing directional casts would have made
        // that scene the one case where shadows silently do not exist.
        let spots =
            kooch_lighting::shadow_casting_spots(resources, kooch_lighting::MAX_SPOT_SHADOWS);
        // Point lights, likewise (#778) — and nearest-first, because
        // past the limit a light stops casting and which one should not
        // depend on spawn order.
        //
        // 🔴 Culled against the camera's frustum BEFORE the limit is
        // applied, not after. Six faces is the most expensive shadow in
        // the engine and `PointLight::cast_shadows` defaults to true, so
        // a corridor of lamps behind the camera would otherwise rasterise
        // twenty-four faces of geometry nobody can see. Culling first
        // also means the four cubes go to lights that are actually on
        // screen rather than to whichever four are nearest including the
        // ones behind you.
        //
        // A light is a sphere of `range`: past that it contributes
        // nothing, so a sphere fully outside the frustum cannot shadow
        // any visible pixel. It CAN shadow a pixel while its own centre
        // is off screen, which is why this is the sphere test and not a
        // point test.
        let frustum = extract_frustum_planes(camera.view_proj(aspect));
        let ranked =
            kooch_lighting::shadow_casting_points(resources, camera.position(), usize::MAX);
        let points = crate::shadow::select_point_casters(
            &ranked,
            &frustum,
            kooch_lighting::MAX_POINT_SHADOWS,
        );

        // 🔴 The cap degrades in silence otherwise. A light past the
        // budget keeps lighting the scene and stops casting, which is
        // the right failure — but an author looking at a lamp whose
        // shadow is missing has no way to tell that from a bug. Logged
        // on the transition only, the way the light count is: the state
        // it reports is steady and sixty lines a second would bury it.
        let visible = ranked
            .iter()
            .filter(|light| {
                !crate::meshlet::sphere_outside_frustum(&frustum, light.position, light.range)
            })
            .count();
        let dropped = visible.saturating_sub(points.len());
        if dropped != self.point_shadows_dropped {
            if dropped > 0 {
                tracing::warn!(
                    target: "kooch_render::shadow",
                    dropped,
                    budget = kooch_lighting::MAX_POINT_SHADOWS,
                    "more point lights are casting than there are cube maps; the ones \
                     furthest from the camera light the scene without a shadow",
                );
            }
            self.point_shadows_dropped = dropped;
        }

        // Release the atlas when it stops being wanted, or when it was
        // allocated at a resolution the author has since changed. Sixty
        // -four megabytes is worth noticing a settings change over, and
        // a texture cannot be resized in place.
        let texels = settings.clamped_texels();
        let nothing_casts = sun.is_none() && spots.is_empty() && points.is_empty();
        if !settings.enabled || nothing_casts || self.shadow_texels != texels {
            if let Some(released) = self.shadows.take() {
                if let Some(tracker) = self.vram_tracker.as_ref() {
                    tracker.sub(released.atlas_bytes());
                }
                tracing::debug!(
                    target: "kooch_render::shadow",
                    "released the shadow atlas",
                );
            }
            self.shadow_texels = 0;
        }
        if !settings.enabled || nothing_casts {
            return None;
        }
        // No sun is not no shadows any more. A scene with only spot
        // lights fits no cascades and still renders their maps; the
        // cascades' own `shadows_enabled` flag stays off, which is what
        // stops the shading model from sampling four empty layers.
        let cascades_enabled = sun.is_some();
        let sun = sun.unwrap_or(glam::Vec3::NEG_Y);

        let shadows = match self.shadows.as_mut() {
            Some(pass) => pass,
            None => {
                tracing::debug!(
                    target: "kooch_render::shadow",
                    cascade_texels = texels,
                    "allocating the shadow atlas",
                );
                let pass = ShadowPass::new(
                    device,
                    self.cull_pipelines.meshlet_bind_group_layout(),
                    texels,
                    self.config.meshlet_capacity,
                    super::super::super::DEFAULT_MAX_TRIANGLES as u32,
                );
                if let Some(tracker) = self.vram_tracker.as_ref() {
                    tracker.add(pass.atlas_bytes());
                }
                self.shadow_texels = texels;
                self.shadows.insert(pass)
            }
        };

        // Binding is idempotent and lives here rather than at
        // allocation: growing the light buffer rebuilds the bind group,
        // and this is the one call site that runs after every possible
        // rebuild.
        let atlas_view = shadows.atlas_view().clone();
        let cubes_view = shadows.cubes_view().clone();
        let prepared = shadows.prepare(
            device,
            camera,
            aspect,
            sun,
            cascades_enabled,
            settings.max_distance,
            settings.first_cascade_distance,
            settings.sun_softness,
            &spots,
            &points,
            meshlet_capacity,
            group_capacity,
        );
        self.lights
            .bind_shadow_maps(device, &atlas_view, &cubes_view);
        Some(prepared)
    }

    /// Records the cascade culls and depth passes into this frame's
    /// encoder, ahead of anything that samples them.
    pub(in crate::meshlet::render_stage) fn record_shadows(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        prepared: &PreparedShadows,
        meshlet_bg: &wgpu::BindGroup,
        instance_count: u32,
        max_meshlets_per_mesh: u32,
        lod_target: f32,
    ) {
        // 🔴 Which cubes still hold last frame's truth.
        //
        // Six faces per light is the most expensive shadow the engine
        // draws, and a lamp bolted to a wall in a room where nothing
        // moves redraws all six of them sixty times a second for no
        // reason. Epic measures a cached local shadow map at 0.05 ms
        // against 0.4-0.8 ms invalidated, on a PS5.
        //
        // The key is deliberately coarse: the light's identity, its
        // position, and a hash of EVERY instance in the frame. A crate
        // moving across the level invalidates a lamp that cannot see it.
        // That is the safe direction — a cube redrawn for nothing costs
        // a frame's work, and a cube not redrawn when it should have
        // been is a shadow frozen in place, which is silent and gets
        // blamed on everything else first. Narrowing it means asking
        // which instances a light's range reaches, and that is the
        // cluster structure (#780), not this issue.
        let scene_hash = self.scene_hash;
        let mut redraw = Vec::new();
        for (slot, draw) in prepared.points.iter().enumerate() {
            let key = draw.key(scene_hash);
            if self.point_cube_cache.get(slot).copied().flatten() != Some(key) {
                redraw.push((slot, *draw));
            }
        }
        // Slots past the live count hold a cube for a light that is no
        // longer casting. Forgetting them is what makes a lamp that
        // stops casting and starts again reuse a stale cube.
        self.point_cube_cache
            .resize(kooch_lighting::MAX_POINT_SHADOWS, None);
        for slot in 0..kooch_lighting::MAX_POINT_SHADOWS {
            self.point_cube_cache[slot] =
                prepared.points.get(slot).map(|draw| draw.key(scene_hash));
        }

        let (Some(shadows), Some(pool)) = (self.shadows.as_ref(), self.gpu_pool.as_ref()) else {
            return;
        };
        shadows.record(
            device,
            queue,
            encoder,
            prepared,
            &redraw,
            &self.cull_pipelines,
            pool,
            &self.scene,
            meshlet_bg,
            instance_count,
            max_meshlets_per_mesh,
            lod_target,
        );
    }
}

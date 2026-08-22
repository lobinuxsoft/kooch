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

use crate::shadow::{CubeKey, PreparedShadows, ShadowPass, ShadowSettings};
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
        lights: &kooch_lighting::LightFrame,
    ) -> Option<PreparedShadows> {
        profiling::scope!("shadows: prepare");
        let settings = resources
            .get::<ShadowSettings>()
            .copied()
            .unwrap_or_default();
        let sun = lights.sun();
        // Spot lights keep the array alive on their own (#777): a scene
        // lit by a torch and no sun still casts, and releasing the
        // texture because nothing directional casts would have made
        // that scene the one case where shadows silently do not exist.
        // Already capped at the budget during the walk, and numbered in
        // the order the slots were handed out — there is no second place
        // that decides which spots fit.
        let spots = lights.spot_shadows().to_vec();
        // Point lights, likewise (#778) — ranked by what a cube would
        // show, because past the limit a light stops casting and which
        // one should not depend on spawn order.
        //
        // 🔴 No camera frustum here any more. It used to cull lamps whose
        // `range` sphere fell outside this camera before the limit was
        // applied, to keep a corridor of lamps behind the viewer from
        // rasterising twenty-four faces nobody can see.
        //
        // A cube map is drawn from the LIGHT, so what it holds cannot
        // depend on where anyone stands — and this function runs once per
        // VIEW while the cubes, the cache and the holders below belong to
        // the stage. The editor renders two views through one stage, so a
        // lamp outside the gameplay camera lost its cube for both panels
        // and the one looking straight at it drew no shadow. Whichever
        // view rendered last decided.
        //
        // The optimisation is still worth having, but it has to be asked
        // of the frame — the union of every active view's frustum, or one
        // selection reused by all of them — not of whoever is rendering.
        let ranked = lights.ranked_points(camera.position(), usize::MAX);
        let points = crate::shadow::select_point_casters(
            &ranked,
            settings.point_budget(),
            &self.point_shadow_holders,
        );
        // Next frame's hysteresis is this frame's answer. Written even
        // when the list is empty: a light that lost its cube must not be
        // handed the bonus back the moment it returns.
        self.point_shadow_holders.clear();
        self.point_shadow_holders
            .extend(points.iter().map(|light| light.entity));

        // 🔴 The cap degrades in silence otherwise. A light past the
        // budget keeps lighting the scene and stops casting, which is
        // the right failure — but an author looking at a lamp whose
        // shadow is missing has no way to tell that from a bug.
        //
        // Reported on entering the state, once, and not on every change
        // of the overflow. It used to move with the camera — 84 and 96
        // alternating in the roll-a-ball stress scene — because the cull
        // ran before the budget; now the count is a property of the
        // scene, so the line is printed once and stays true.
        let dropped = ranked.len().saturating_sub(points.len());
        if (dropped > 0) != self.point_shadows_over_budget {
            if dropped > 0 {
                tracing::warn!(
                    target: "kooch_render::shadow",
                    dropped,
                    budget = settings.point_budget(),
                    "more point lights are casting than there are cube maps; the ones \
                     furthest from the camera light the scene without a shadow. Logged \
                     once — the count moves with the camera",
                );
            }
            self.point_shadows_over_budget = dropped > 0;
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
        //
        // 🔴 And neither is "the virtual pages are on". `inti_shadow`
        // takes the page branch and never reads a cascade layer, so
        // every cascade cull, every cascade draw and the whole fit were
        // work with no reader — all four layers, every frame, for as
        // long as the feature has existed. The atlas itself STAYS: its
        // trailing layers are the spot lights, which have no page
        // raster yet.
        let cascades_enabled = sun.is_some() && !settings.virtual_pages;
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
                    settings.point_budget() as u32,
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
        profiling::scope!("shadows: record");
        // 🔴 Which cubes still hold last frame's truth.
        //
        // Six faces per light is the most expensive shadow the engine
        // draws, and a lamp bolted to a wall in a room where nothing
        // moves redraws all six of them sixty times a second for no
        // reason. Epic measures a cached local shadow map at 0.05 ms
        // against 0.4-0.8 ms invalidated, on a PS5.
        //
        // The key is the light's identity, its position, and a digest of
        // the instances ITS OWN RANGE reaches (#847). It used to be a
        // hash of every instance in the frame, which meant a crate
        // moving across the level invalidated a lamp that could not see
        // it — +2.0 ms in any scene where anything moves, which is every
        // scene in a game.
        //
        // Still conservative where it counts: a cube redrawn for nothing
        // costs a frame's work, and a cube NOT redrawn when it should
        // have been is a shadow frozen in place, which is silent and
        // gets blamed on everything else first.
        // `light_scene_hash` digests
        // only the instances this lamp's own range can reach, so a crate
        // moving in another room no longer costs six faces here.
        let keys: Vec<CubeKey> = prepared
            .points
            .iter()
            .map(|draw| {
                draw.key(crate::shadow::light_scene_hash(
                    &self.instance_bounds,
                    draw.eye,
                    draw.range,
                ))
            })
            .collect();
        let mut redraw = Vec::new();
        for (slot, draw) in prepared.points.iter().enumerate() {
            if self.point_cube_cache.get(slot).copied().flatten() != Some(keys[slot]) {
                redraw.push((slot, *draw));
            }
        }
        // Slots past the live count hold a cube for a light that is no
        // longer casting. Forgetting them is what makes a lamp that
        // stops casting and starts again reuse a stale cube.
        self.point_cube_cache
            .resize(kooch_lighting::MAX_POINT_SHADOWS, None);
        for slot in 0..kooch_lighting::MAX_POINT_SHADOWS {
            self.point_cube_cache[slot] = keys.get(slot).copied();
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

//! Per-frame lighting constants: what the scene is exposed at, and
//! what light arrives from nowhere in particular.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// Hemisphere ambient standing in for IBL (#450) — without it a metal away from lights is black.
/// Insert one into [`Resources`](kooch_core::resource::Resources) to override.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AmbientLight {
    /// Linear RGB arriving from world up.
    pub sky_color: Vec3,
    /// Linear RGB arriving from world down — bounce, not sky.
    pub ground_color: Vec3,
    /// Illuminance in lux, on the same scale as a `DirectionalLight`.
    pub intensity: f32,
}

impl Default for AmbientLight {
    /// An overcast-ish sky over neutral ground, at roughly 3 % of the
    /// 10 000 lux a default `DirectionalLight` puts out. Enough to read
    /// shape in shadow, far too little to be mistaken for a key light.
    fn default() -> Self {
        Self {
            sky_color: Vec3::new(0.4, 0.55, 0.75),
            ground_color: Vec3::new(0.2, 0.18, 0.15),
            intensity: 300.0,
        }
    }
}

/// Exposure in EV100: lights carry physical units (a sun is 10 000 lux), so without it everything
/// clips. Auto exposure is #254. Prefer [`PhysicalCamera`], whose settings say which way is
/// brighter.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Exposure {
    pub ev100: f32,
}

impl Default for Exposure {
    /// [`PhysicalCamera::default`]'s value, about 9.9 — near Bevy's 9.7, which matches Blender's
    /// implicit exposure.
    fn default() -> Self {
        Self::from_physical(PhysicalCamera::default())
    }
}

impl Exposure {
    /// Radiance multiplier before tonemapping: `1 / (2^EV100 × 1.2)`, the standard meter
    /// calibration constant.
    pub fn multiplier(&self) -> f32 {
        1.0 / (2.0f32.powf(self.ev100) * 1.2)
    }

    /// Exposure for a real camera's settings.
    pub fn from_physical(camera: PhysicalCamera) -> Self {
        Self {
            ev100: camera.ev100(),
        }
    }
}

/// Exposure as aperture, shutter and ISO — controls a person can reason about, as Bevy 0.13 added —
/// until auto exposure (#254) and GI (#450).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PhysicalCamera {
    /// f-stop. Lower is a wider aperture and a brighter image: f/1.4
    /// gathers four times the light of f/2.8.
    pub aperture_f_stops: f32,
    /// Shutter time in seconds. `1.0 / 125.0` is a typical handheld
    /// exposure; longer is brighter.
    pub shutter_speed_s: f32,
    /// Film speed. Higher is brighter, and in a real camera noisier —
    /// here it is brightness only.
    pub sensitivity_iso: f32,
}

impl Default for PhysicalCamera {
    /// f/2.8, 1/125 s, ISO 100 (EV100 ≈ 9.9): a default sun does not clip and punctual lights show.
    /// A renderer need, not a real situation — see the presets.
    fn default() -> Self {
        Self {
            aperture_f_stops: 2.8,
            shutter_speed_s: 1.0 / 125.0,
            sensitivity_iso: 100.0,
        }
    }
}

impl PhysicalCamera {
    /// Sunny 16: f/16, 1/125 s, ISO 100 (EV100 ≈ 15). Pair with `lux::DIRECT_SUNLIGHT`; the 10 000
    /// lux default is daylight, not sun.
    pub fn sunny() -> Self {
        Self {
            aperture_f_stops: 16.0,
            shutter_speed_s: 1.0 / 125.0,
            sensitivity_iso: 100.0,
        }
    }

    #[cfg(test)]
    /// Indoors: f/1.0, 1/125 s, ISO 100 (EV100 ≈ 7), as Bevy's lighting example — eight stops above
    /// [`Self::sunny`].
    pub fn indoor() -> Self {
        Self {
            aperture_f_stops: 1.0,
            shutter_speed_s: 1.0 / 125.0,
            sensitivity_iso: 100.0,
        }
    }

    /// `log2(N² / t) - log2(S / 100)`: aperture and shutter set exposure, sensitivity shifts its
    /// scale.
    pub fn ev100(&self) -> f32 {
        let n = self.aperture_f_stops.max(1e-3);
        let t = self.shutter_speed_s.max(1e-9);
        let s = self.sensitivity_iso.max(1e-3);
        ((n * n) / t).log2() - (s / 100.0).log2()
    }
}

/// One cascade, as the shader reads it. Mirrors `IntiCascade` in
/// `inti_pbr.wgsl`: 96 bytes, and the stride has to stay a multiple of
/// 16 or the array indexes into the middle of the previous entry.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuCascade {
    pub view_proj: [[f32; 4]; 4],
    /// The shadow array layer — dynamic indexing needs no binding arrays, just
    /// `texture_depth_2d_array`, as Bevy.
    pub layer: u32,
    /// 🔴 Three scalars on the WGSL side too, never a `vec3<u32>`: that
    /// aligns to 16 and grows every cascade by 16 bytes, which surfaces
    /// only as `min_binding_size` rejecting the pipeline.
    pub _pad_layer: [u32; 3],
    pub far_depth: f32,
    pub texel_world_size: f32,
    /// World units spanned by [0, 1] depth, so PCSS can measure its gap in metres.
    pub depth_extent: f32,
    pub _pad0: f32,
}

/// Casting spot lights per frame (#777): each is a 2048² `Depth32Float` layer (16 MiB) and a cull.
/// A budget; spots past it still light, just without shadows.
pub const MAX_SPOT_SHADOWS: usize = 4;

/// How many cascades the frame carries. Fixed because the count is baked
/// into the atlas layout — changing it is a texture change.
pub const FRAME_CASCADE_COUNT: usize = 4;

/// Point lights casting at once (#778, raised by #849): each cube is six 512² `Depth32Float` faces,
/// 6 MiB, so memory decides the number. Past it a light still lights without casting; which lights
/// cast is ranked.
pub const MAX_POINT_SHADOWS: usize = 32;

/// What shading needs per point cube (#778): 16 B and no matrix, since cube sampling takes a
/// direction.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuPointShadow {
    /// 🔴 The whole depth reconstruction: with infinite reverse-Z (ADR 0002) Bevy's `zw.x / zw.y` is
    /// `near / major_axis`.
    pub near: f32,
    /// Shadow-texel size **per metre of distance from the light**, the
    /// way a spot's is — a cube face is a 90° perspective, so this is
    /// `2 / size` and never involves `range`.
    pub texel_world_size: f32,
    /// World units the usable depth range spans, for the penumbra
    /// estimate. The light's range.
    pub depth_extent: f32,
    pub _pad0: f32,
}

/// Mirror of `IntiFrame` in `inti_pbr.wgsl`, size pinned in `frame/tests.rs`. `camera_position` is
/// here because the camera UBO is pinned at 64 B; per-view data in a shared binding is safe because
/// each view submits its own encoder.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct IntiFrame {
    pub ambient_sky: [f32; 3],
    pub light_count: u32,
    pub ambient_ground: [f32; 3],
    pub exposure: f32,
    pub camera_position: [f32; 3],
    pub ambient_intensity: f32,
    /// Unit view axis for the cascades, whose boundaries must be planes, not radial spheres.
    pub camera_forward: [f32; 3],
    pub _pad_forward: f32,
    pub cascades: [GpuCascade; FRAME_CASCADE_COUNT],
    /// One per casting spot (#777) in the cascade record: `inti_shadow_coords` already divides by
    /// `w`, so bias, Castano and clamp are shared. Bevy rebuilds the basis in the shader only
    /// because its record has no room.
    pub spot_shadows: [GpuCascade; MAX_SPOT_SHADOWS],
    /// How many entries of `spot_shadows` are live this frame.
    pub spot_shadow_count: u32,
    /// Live `point_shadows` entries, riding a pad word the spot count left.
    pub point_shadow_count: u32,
    /// Irradiance below which a light pays diffuse only (#821) — the specular half is the expensive
    /// one, wasted on invisible highlights. **0.0 keeps the full model.**
    pub specular_floor: f32,
    /// How many of a froxel's punctual lights a pixel evaluates; **0 = all**. 🔴 A measuring
    /// instrument: it drops real light, but answers whether cost follows the 12–15 lights that
    /// reach a pixel (#820), deciding #825 vs #826. Directional lights uncounted.
    pub light_limit: u32,
    /// One per casting point light (#778) — its own record, since a cube is sampled by direction
    /// and `GpuCascade`'s matrix would be dead weight.
    pub point_shadows: [GpuPointShadow; MAX_POINT_SHADOWS],
    /// 0 when nothing casts, or the atlas has not been rendered. The
    /// dummy atlas bound in that case reads as fully lit anyway; the
    /// flag skips the sampling.
    pub shadows_enabled: u32,
    /// Fraction of a split distance over which one cascade fades into
    /// the next.
    pub cascade_blend: f32,
    /// Tangent of the sun's angular radius — shadow widening per metre of gap; an angle because the
    /// sun is infinitely far.
    pub sun_softness: f32,
    /// The single-light view's buffer index (#743), `>= light_count` for none (see
    /// [`NO_DEBUG_LIGHT`]); in former tail padding, since Inti's group is full.
    pub debug_light: u32,
    /// The view matrix's third row: shading needs only view z, for the froxel slice.
    pub view_z_row: [f32; 4],
    /// xyz = the grid's dimensions, w = their product.
    pub cluster_dimensions: [u32; 4],
    /// xy = grid cells per pixel, zw = the logarithmic slice constants.
    pub cluster_factors: [f32; 4],
    /// Index-list length for the loop to clamp against; an overflowed frame renders under-lit, not
    /// garbage.
    pub cluster_capacity: u32,
    /// Directional lights, unclustered: the first `directional_count` buffer entries, walked
    /// linearly.
    pub directional_count: u32,
    /// 0 while no grid has been built — an unclustered frame walks every
    /// light the way it did before #780, which is what the headless
    /// tests and any path with no camera matrices do.
    pub clustered: u32,
    /// Count at which `LightsPerPixel` reads full red (#817) — a uniform, since a useful top
    /// depends on the scene. Zero reads as [`LIGHTS_HOT_DEFAULT`].
    pub debug_lights_hot: u32,
    /// To 16: `debug_lights_hot` closed a group of four. Four pad words since #826's
    /// `light_samples` left, ready for the next scalar.
    pub _pad_samples: [u32; 4],
}

/// Initial top of scale for the lights-per-pixel view: sixteen quarters readably; the editor moves
/// it.
pub const LIGHTS_HOT_DEFAULT: u32 = 16;

/// Irradiance below which a light skips specular (#821), as a
/// [`Resource`](kooch_core::resource::Resources). `0.0` renders as before; the value is found by
/// sweeping.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SpecularFloor(pub f32);

impl Default for SpecularFloor {
    fn default() -> Self {
        Self(floor_from_environment())
    }
}

/// `KOOCH_SPECULAR_FLOOR=<lux>`, read once. 🔴 An environment variable because it can only be
/// measured on the OneXFly over SSH — the desktop raster pass is 0.12 ms. Unparseable keeps the
/// default.
fn floor_from_environment() -> f32 {
    static FLOOR: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *FLOOR.get_or_init(|| {
        let Ok(raw) = std::env::var("KOOCH_SPECULAR_FLOOR") else {
            return 0.0;
        };
        match raw.trim().parse::<f32>() {
            Ok(floor) if floor >= 0.0 => {
                tracing::info!(
                    "KOOCH_SPECULAR_FLOOR={floor}: lights under this irradiance shade \
                     diffuse-only"
                );
                floor
            }
            _ => {
                tracing::warn!("KOOCH_SPECULAR_FLOOR={raw:?} is not a number — keeping 0");
                0.0
            }
        }
    })
}

/// Punctual lights per pixel as a [`Resource`](kooch_core::resource::Resources), `0` = all; see
/// [`IntiFrame::light_limit`]. An environment variable too, since only the handheld can measure it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LightLimit(pub u32);

impl Default for LightLimit {
    fn default() -> Self {
        Self(limit_from_environment())
    }
}

/// `KOOCH_LIGHT_LIMIT=<n>`, read once; unparseable or negative keeps "all" so a typo cannot change
/// a measurement.
fn limit_from_environment() -> u32 {
    static LIMIT: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        let Ok(raw) = std::env::var("KOOCH_LIGHT_LIMIT") else {
            return 0;
        };
        match raw.trim().parse::<u32>() {
            Ok(limit) => {
                tracing::info!(
                    "KOOCH_LIGHT_LIMIT={limit}: a pixel evaluates at most this many of its \
                     froxel's punctual lights. The picture is wrong on purpose — this \
                     measures whether the cost scales with that count."
                );
                limit
            }
            Err(_) => {
                tracing::warn!("KOOCH_LIGHT_LIMIT={raw:?} is not a count — keeping all lights");
                0
            }
        }
    })
}

/// Top of scale for `LightsPerPixel`, a [`Resource`](kooch_core::resource::Resources) the editor
/// writes, like [`DebugLight`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LightsHot(pub u32);

impl Default for LightsHot {
    fn default() -> Self {
        Self(LIGHTS_HOT_DEFAULT)
    }
}

/// [`IntiFrame::debug_light`] when no light is isolated. Any index past
/// the light count reads the same way; this one is the deliberate value.
pub const NO_DEBUG_LIGHT: u32 = u32::MAX;

/// The light `SingleLight` isolates (#743), written by the editor from the World panel selection.
/// `None` or a non-light renders magenta.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct DebugLight(pub Option<kooch_ecs::entity::Entity>);

impl IntiFrame {
    pub fn new(
        ambient: &AmbientLight,
        exposure: &Exposure,
        camera_position: Vec3,
        light_count: u32,
    ) -> Self {
        Self {
            ambient_sky: ambient.sky_color.to_array(),
            light_count,
            ambient_ground: ambient.ground_color.to_array(),
            exposure: exposure.multiplier(),
            camera_position: camera_position.to_array(),
            ambient_intensity: ambient.intensity,
            camera_forward: Vec3::NEG_Z.to_array(),
            _pad_forward: 0.0,
            cascades: [GpuCascade::default(); FRAME_CASCADE_COUNT],
            spot_shadows: [GpuCascade::default(); MAX_SPOT_SHADOWS],
            spot_shadow_count: 0,
            point_shadow_count: 0,
            specular_floor: 0.0,
            light_limit: 0,
            point_shadows: [GpuPointShadow::default(); MAX_POINT_SHADOWS],
            shadows_enabled: 0,
            cascade_blend: 0.1,
            sun_softness: DEFAULT_SUN_SOFTNESS,
            debug_light: NO_DEBUG_LIGHT,
            view_z_row: [0.0, 0.0, -1.0, 0.0],
            cluster_dimensions: [0; 4],
            cluster_factors: [0.0; 4],
            cluster_capacity: 0,
            directional_count: 0,
            clustered: 0,
            debug_lights_hot: LIGHTS_HOT_DEFAULT,
            _pad_samples: [0; 4],
        }
    }

    /// How many of the buffer's leading entries are directional lights.
    pub fn with_directionals(mut self, count: u32) -> Self {
        self.directional_count = count;
        self
    }

    /// Points shading at this view's grid (#780); absent, the loop walks every light.
    pub fn with_clusters(mut self, grid: &crate::ClusterGrid, view: Mat4, capacity: u32) -> Self {
        let dims = grid.dimensions;
        self.view_z_row = view.row(2).to_array();
        self.cluster_dimensions = [dims.x, dims.y, dims.z, grid.cluster_count()];
        self.cluster_factors = [
            grid.tile_factors.x,
            grid.tile_factors.y,
            grid.z_factors.x,
            grid.z_factors.y,
        ];
        self.cluster_capacity = capacity;
        self.clustered = 1;
        self
    }

    /// Isolates one light for the single-light debug view (#743).
    /// `None` — or an index the buffer does not hold — shows nothing.
    pub fn with_debug_light(mut self, index: Option<u32>) -> Self {
        self.debug_light = index.unwrap_or(NO_DEBUG_LIGHT);
        self
    }

    /// Irradiance below which a light skips specular (#821), clamped at zero, which already means
    /// never skip.
    pub fn with_specular_floor(mut self, floor: f32) -> Self {
        self.specular_floor = floor.max(0.0);
        self
    }

    /// Caps how many of a froxel's punctual lights a pixel evaluates
    /// (0 = all). See [`IntiFrame::light_limit`].
    pub fn with_light_limit(mut self, limit: u32) -> Self {
        self.light_limit = limit;
        self
    }

    /// Sets the lights-per-pixel view's top of scale (#817), at least one: zero would paint the
    /// whole screen hot.
    pub fn with_lights_hot(mut self, hot: u32) -> Self {
        self.debug_lights_hot = hot.max(1);
        self
    }

    /// Attaches the shadows from [`FrameShadows`], if the frame has any.
    pub fn with_optional_shadows(self, shadows: Option<FrameShadows>) -> Self {
        match shadows {
            Some(s) => {
                let frame = if s.cascades_enabled {
                    self.with_shadows(s.camera_forward, s.cascades, s.blend, s.sun_softness)
                } else {
                    self
                };
                frame
                    .with_spot_shadows(s.spot_shadows, s.spot_shadow_count)
                    .with_point_shadows(s.point_shadows, s.point_shadow_count)
            }
            None => self,
        }
    }

    /// Attaches the shadow cascades and turns sampling on.
    pub fn with_shadows(
        mut self,
        camera_forward: Vec3,
        cascades: [GpuCascade; FRAME_CASCADE_COUNT],
        blend: f32,
        sun_softness: f32,
    ) -> Self {
        self.camera_forward = camera_forward.normalize_or(Vec3::NEG_Z).to_array();
        self.cascades = cascades;
        self.shadows_enabled = 1;
        self.cascade_blend = blend;
        self.sun_softness = sun_softness.max(0.0);
        self
    }

    /// Attaches spot shadow maps (#777), separate from [`Self::with_shadows`]: that flag gates
    /// cascades, and a spot can cast with no sun.
    pub fn with_spot_shadows(
        mut self,
        spot_shadows: [GpuCascade; MAX_SPOT_SHADOWS],
        count: u32,
    ) -> Self {
        self.spot_shadows = spot_shadows;
        self.spot_shadow_count = count.min(MAX_SPOT_SHADOWS as u32);
        self
    }

    /// Attaches the point lights' cube maps (#778). Independent of the
    /// cascades for the same reason the spots' are.
    pub fn with_point_shadows(
        mut self,
        point_shadows: [GpuPointShadow; MAX_POINT_SHADOWS],
        count: u32,
    ) -> Self {
        self.point_shadows = point_shadows;
        self.point_shadow_count = count.min(MAX_POINT_SHADOWS as u32);
        self
    }
}

/// Everything the frame samples shadows with, produced by `kooch_render`. One value, because
/// cascades from one camera with another's axis misplace every boundary.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FrameShadows {
    /// Unit vector down the view axis, for the cascade selector.
    pub camera_forward: Vec3,
    pub cascades: [GpuCascade; FRAME_CASCADE_COUNT],
    /// Fraction of a split distance the cascades cross-fade over.
    pub blend: f32,
    /// Tangent of the sun's angular radius. See [`IntiFrame::sun_softness`].
    pub sun_softness: f32,
    /// 🔴 Whether cascades are real: false when only spots cast, where cascades are fitted to a
    /// stand-in direction a non-casting sun must not sample.
    pub cascades_enabled: bool,
    /// One per shadow-casting spot light (#777).
    pub spot_shadows: [GpuCascade; MAX_SPOT_SHADOWS],
    /// How many of `spot_shadows` are live.
    pub spot_shadow_count: u32,
    /// One per shadow-casting point light (#778).
    pub point_shadows: [GpuPointShadow; MAX_POINT_SHADOWS],
    /// How many of `point_shadows` are live.
    pub point_shadow_count: u32,
    /// The entity per live cube, in slot order. 🔴 Carried, not recomputed: slots are ranked by
    /// importance, not walk order, and two rankings would drift.
    pub point_entities: [kooch_ecs::entity::Entity; MAX_POINT_SHADOWS],
}

/// Default sun softness: the real 0.0047 looks like PCF at eight taps' cost; 0.03, a three-degree
/// sun, gives ~7 cm of penumbra per metre — attached at the base, soft further out.
pub const DEFAULT_SUN_SOFTNESS: f32 = 0.03;

#[cfg(test)]
mod tests;

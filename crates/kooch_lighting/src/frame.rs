//! Per-frame lighting constants: what the scene is exposed at, and
//! what light arrives from nowhere in particular.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// Hemisphere ambient — the stand-in for image-based lighting until
/// #450 lands a real probe.
///
/// Not cosmetic. With no ambient term a metal has no environment to
/// reflect, so every metallic surface not facing a light renders pure
/// black: correct for the model, and indistinguishable from a bug to
/// whoever is looking at it.
///
/// Insert one into [`Resources`](kooch_core::resource::Resources) to
/// override; absent, the default below is used.
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

/// Camera exposure, in the photographic EV100 scale.
///
/// The lights carry physical units — a `DirectionalLight` defaults to
/// 10 000 lux — so without an exposure step every channel clips to
/// white and the shading model looks broken rather than unexposed.
/// This is the fixed stand-in; #254 owns auto exposure, which stops
/// being cosmetic at planetary scale where a sunlit surface and the
/// night side differ by orders of magnitude.
///
/// # Prefer [`PhysicalCamera`]
///
/// `EV100 = 9.7` is a correct number and an unusable control: nothing
/// about it says which way is brighter or how much a step is worth.
/// `f/16, 1/125 s, ISO 100` says the same thing to anyone who has held
/// a camera. [`PhysicalCamera::ev100`] converts.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Exposure {
    pub ev100: f32,
}

impl Default for Exposure {
    /// Whatever [`PhysicalCamera::default`] works out to — one source of
    /// truth rather than a bare number that has to be kept in step with
    /// the camera settings that are supposed to explain it.
    ///
    /// It lands near 9.9, which is close to Bevy's 9.7. Theirs is not
    /// "sunny 16" despite how it is often described; they calibrated it
    /// to match Blender's implicit exposure. A quarter of a stop apart
    /// means a scene authored against their numbers reads the same here.
    fn default() -> Self {
        Self::from_physical(PhysicalCamera::default())
    }
}

impl Exposure {
    /// The multiplier the shader applies to radiance before tonemapping.
    ///
    /// `1 / (2^EV100 × 1.2)`: the 1.2 is the standard reflected-light
    /// meter calibration constant, not a fudge factor.
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

/// A real camera's settings, as the way to say how bright the scene
/// should look.
///
/// Aperture, shutter and ISO are three numbers a person can reason
/// about — open the aperture, get more light — where EV100 is one number
/// that reasons about nothing. Bevy added the same thing in 0.13 for the
/// same reason.
///
/// This is the honest half of the fix for physical light units being
/// unusable. The other halves are auto exposure (#254) and global
/// illumination (#450); until those, an author who finds the scene too
/// dark has a control that behaves the way they expect.
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
    /// f/2.8 at 1/125 s, ISO 100 — EV100 ≈ 9.9.
    ///
    /// A middle setting rather than a real situation: bright enough that
    /// a default `DirectionalLight` does not clip and dim enough that a
    /// punctual light is visible. Neither of those is a photographic
    /// fact; they are what this renderer needs while it has no global
    /// illumination, and the presets below are the real situations.
    fn default() -> Self {
        Self {
            aperture_f_stops: 2.8,
            shutter_speed_s: 1.0 / 125.0,
            sensitivity_iso: 100.0,
        }
    }
}

impl PhysicalCamera {
    /// Bright sun outdoors: f/16, 1/125 s, ISO 100 — "sunny 16",
    /// EV100 ≈ 15.
    ///
    /// Pair it with `lux::DIRECT_SUNLIGHT` on the directional light.
    /// Used with a 10 000 lux default sun, the scene comes out dark,
    /// which is correct: 10 000 lux is ambient daylight, not sun.
    pub fn sunny() -> Self {
        Self {
            aperture_f_stops: 16.0,
            shutter_speed_s: 1.0 / 125.0,
            sensitivity_iso: 100.0,
        }
    }

    /// Indoors under artificial light: f/1.0, 1/125 s, ISO 100 —
    /// EV100 ≈ 7. The same settings Bevy's lighting example uses.
    ///
    /// About eight stops brighter than [`Self::sunny`], which is roughly
    /// the gap between a sunlit exterior and a lit room — the gap that
    /// makes a physically-correct bulb look like nothing.
    pub fn indoor() -> Self {
        Self {
            aperture_f_stops: 1.0,
            shutter_speed_s: 1.0 / 125.0,
            sensitivity_iso: 100.0,
        }
    }

    /// The equivalent EV100.
    ///
    /// `log2(N² / t) - log2(S / 100)`, the standard photographic
    /// relation: aperture and shutter set the exposure, sensitivity
    /// shifts the scale it is measured against.
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
    /// xy scale, zw bias — maps this cascade's `[0,1]` uv into the atlas.
    pub uv_scale_bias: [f32; 4],
    pub far_depth: f32,
    pub texel_world_size: f32,
    /// World units the `[0,1]` depth range spans, so the shader can turn
    /// a difference between two stored depths into metres. PCSS's
    /// penumbra is proportional to that distance, and a ratio with no
    /// scale is only usable through a constant that is wrong in three
    /// cascades out of four.
    pub depth_extent: f32,
    pub _pad0: f32,
}

/// How many cascades the frame carries. Fixed because the count is baked
/// into the atlas layout — changing it is a texture change.
pub const FRAME_CASCADE_COUNT: usize = 4;

/// Mirror of `IntiFrame` in `inti_pbr.wgsl`. 464 bytes.
///
/// `camera_position` rides here rather than in the shared camera UBO
/// because that UBO is pinned at 64 B by two bind-group layouts, and
/// widening it would ripple through paths this work has no business
/// touching. It also makes this struct the one per-view thing in an
/// otherwise per-frame binding — see [`crate::GpuLights::write_frame`]
/// for why that is safe with more than one view.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct IntiFrame {
    pub ambient_sky: [f32; 3],
    pub light_count: u32,
    pub ambient_ground: [f32; 3],
    pub exposure: f32,
    pub camera_position: [f32; 3],
    pub ambient_intensity: f32,
    /// Unit vector down the view axis. Only the cascades use it, and
    /// they need the axis rather than the radial direction: a radial
    /// distance makes every cascade boundary a sphere, which crosses in
    /// the corners of the screen before the centre.
    pub camera_forward: [f32; 3],
    pub _pad_forward: f32,
    pub cascades: [GpuCascade; FRAME_CASCADE_COUNT],
    /// 0 when nothing casts, or the atlas has not been rendered. The
    /// dummy atlas bound in that case reads as fully lit anyway; the
    /// flag skips the sampling.
    pub shadows_enabled: u32,
    /// Fraction of a split distance over which one cascade fades into
    /// the next.
    pub cascade_blend: f32,
    /// Tangent of the sun's angular RADIUS — how much wider a shadow
    /// gets per metre between blocker and receiver.
    ///
    /// An angle rather than a width, because that is what a light
    /// infinitely far away has. A width in world units would have to
    /// mean a width *at some distance*, and no code path was ever going
    /// to agree on which.
    pub sun_softness: f32,
    pub _pad_frame: f32,
}

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
            shadows_enabled: 0,
            cascade_blend: 0.1,
            sun_softness: DEFAULT_SUN_SOFTNESS,
            _pad_frame: 0.0,
        }
    }

    /// Attaches the shadows from [`FrameShadows`], if the frame has any.
    pub fn with_optional_shadows(self, shadows: Option<FrameShadows>) -> Self {
        match shadows {
            Some(s) => self.with_shadows(s.camera_forward, s.cascades, s.blend, s.sun_softness),
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
}

/// Everything the frame needs to sample shadows, as one value.
///
/// The producer is `kooch_render` — placing cascades needs the meshlet
/// pipeline's atlas, and this crate sits below it. Grouped rather than
/// passed as three parameters because they are only ever correct
/// together: cascades from one camera with the forward axis of another
/// puts every cascade boundary in the wrong place, and three loose
/// arguments is how that happens.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FrameShadows {
    /// Unit vector down the view axis, for the cascade selector.
    pub camera_forward: Vec3,
    pub cascades: [GpuCascade; FRAME_CASCADE_COUNT],
    /// Fraction of a split distance the cascades cross-fade over.
    pub blend: f32,
    /// Tangent of the sun's angular radius. See [`IntiFrame::sun_softness`].
    pub sun_softness: f32,
}

/// Tangent of the sun's angular radius, by default.
///
/// The real sun subtends about half a degree, so the honest value is
/// 0.0047 — and at that width PCSS is indistinguishable from PCF and
/// costs eight extra taps to prove it. 0.03 is roughly a three-degree
/// sun: about seven centimetres of penumbra per metre of gap, which is
/// what makes a shadow read as attached at its base and soft where it
/// is not. Every film and game widens it, for this reason.
pub const DEFAULT_SUN_SOFTNESS: f32 = 0.03;

#[cfg(test)]
mod tests;

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

/// Mirror of `IntiFrame` in `inti_pbr.wgsl`. 48 bytes.
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_size_matches_shader() {
        assert_eq!(std::mem::size_of::<IntiFrame>(), 48);
    }

    #[test]
    fn default_exposure_brings_a_default_sun_into_range() {
        // 10 000 lux × exposure, through a Lambertian white surface
        // facing the light, must land near 1.0 rather than clipping by
        // an order of magnitude. This is the assertion that catches
        // "the whole scene is a white rectangle" before a smoke test
        // has to.
        let peak = 10_000.0 * Exposure::default().multiplier() / std::f32::consts::PI;
        assert!(
            (0.5..8.0).contains(&peak),
            "peak diffuse response was {peak}, tonemapping cannot rescue that",
        );
    }

    #[test]
    fn exposure_is_monotonic_in_ev100() {
        assert!(Exposure { ev100: 8.0 }.multiplier() > Exposure { ev100: 12.0 }.multiplier());
    }

    /// Swapping one control for the other must not change the picture
    /// until someone turns a dial.
    #[test]
    fn the_two_exposure_controls_agree_by_construction() {
        assert_eq!(
            Exposure::default().ev100,
            PhysicalCamera::default().ev100(),
            "the default exposure and the default camera describe the same light",
        );
    }

    /// The presets are named after real situations, so the arithmetic
    /// has to land where photography says it does — otherwise the names
    /// are decoration.
    #[test]
    fn the_presets_land_on_their_photographic_values() {
        let sunny = PhysicalCamera::sunny().ev100();
        let indoor = PhysicalCamera::indoor().ev100();
        assert!(
            (sunny - 15.0).abs() < 0.2,
            "sunny 16 is EV100 15, got {sunny}",
        );
        assert!(
            (indoor - 7.0).abs() < 0.2,
            "a lit interior is EV100 7, got {indoor}",
        );
    }

    /// Kept close to Bevy's 9.7 so a scene authored against their
    /// numbers reads the same here. Their value is not "sunny 16"
    /// despite how it is usually described — they matched Blender.
    #[test]
    fn the_default_stays_within_a_stop_of_bevys() {
        assert!(
            (Exposure::default().ev100 - 9.7).abs() < 1.0,
            "drifted to {}, and a scene ported from Bevy will not match",
            Exposure::default().ev100,
        );
    }

    #[test]
    fn opening_the_aperture_brightens_the_image() {
        let wide = Exposure::from_physical(PhysicalCamera {
            aperture_f_stops: 1.4,
            ..Default::default()
        });
        let narrow = Exposure::from_physical(PhysicalCamera::default());
        assert!(
            wide.multiplier() > narrow.multiplier(),
            "a wider aperture has to let in more light, or the control lies",
        );
    }

    /// The gap that makes a physically-correct bulb look like nothing.
    #[test]
    fn the_indoor_preset_is_several_stops_brighter_than_sunlight() {
        let stops = PhysicalCamera::sunny().ev100() - PhysicalCamera::indoor().ev100();
        assert!(
            (6.0..10.0).contains(&stops),
            "indoor is {stops} stops from sunlight, which is not the gap it is for",
        );
    }

    #[test]
    fn degenerate_camera_settings_do_not_produce_nan() {
        let broken = PhysicalCamera {
            aperture_f_stops: 0.0,
            shutter_speed_s: 0.0,
            sensitivity_iso: 0.0,
        };
        assert!(broken.ev100().is_finite(), "got {}", broken.ev100());
    }
}

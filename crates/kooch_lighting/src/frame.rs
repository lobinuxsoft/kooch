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
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Exposure {
    pub ev100: f32,
}

impl Default for Exposure {
    /// EV100 9.7 — the "sunny 16" daylight exposure, and the value
    /// Bevy defaults to, so a scene authored against their numbers
    /// reads the same here.
    fn default() -> Self {
        Self { ev100: 9.7 }
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
}

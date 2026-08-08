//! Sky renderer component.
//!
//! Tags an entity as a sky / background source for the scene. The render
//! pipeline iterates entities with `SkyRenderer`, filters by `active`,
//! and picks the highest-`priority` one as the sky for the frame.
//!
//! Multiple `SkyRenderer` entities may coexist in a scene (presets,
//! day/night variants) but only one renders per frame per camera.
//!
//! Parameters are stored inline on the component for MVP. When the asset
//! handle system lands (see tracking issue), a `material: String` field
//! pointing to a `.sky_material` asset will replace the inline fields.

use glam::Vec3;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Component that marks an entity as a sky background source.
///
/// Sky pipeline: procedural vertical gradient (horizon → zenith) with
/// optional volumetric clouds rendered as a slab between `cloud_height`
/// and `cloud_height + cloud_thickness`. Clouds use a fragment-only
/// ray-march with hash-based noise + FBM, Beer-Lambert absorption, and
/// Henyey-Greenstein phase for sun in-scattering.
///
/// Set `cloud_coverage = 0.0` to disable clouds entirely (the sky
/// renders as a pure gradient).
///
/// # Default
///
/// - `active`: true
/// - `priority`: 0
/// - `top_color`: light blue `(0.5, 0.7, 1.0)`
/// - `bottom_color`: dark blue `(0.1, 0.2, 0.4)`
/// - `sun_direction`: up-forward `(0.3, 0.7, -0.5)` (normalized in-shader)
/// - `sun_color`: warm white `(1.0, 0.95, 0.85)`
/// - `cloud_coverage`: 0.45 (medium overcast)
/// - `cloud_density`: 0.8
/// - `cloud_height`: 80.0 (world units above origin)
/// - `cloud_thickness`: 60.0
/// - `wind_direction`: `(1.0, 0.0, 0.3)`
/// - `wind_speed`: 2.0 (world units per second)
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Rendering")]
pub struct SkyRenderer {
    /// Whether this sky is active. Inactive skies are ignored by the renderer.
    pub active: bool,
    /// Priority for multi-sky tiebreaking (higher wins).
    pub priority: i32,
    /// Zenith color (top of the dome) in linear RGB.
    pub top_color: Vec3,
    /// Horizon / nadir color (bottom of the dome) in linear RGB.
    pub bottom_color: Vec3,
    /// Direction the sun rays travel FROM (world-space). Normalized in-shader.
    pub sun_direction: Vec3,
    /// Sun color / tint in linear RGB (multiplies in-scattering).
    pub sun_color: Vec3,
    /// Cloud coverage in `[0, 1]`. `0` disables clouds entirely.
    pub cloud_coverage: f32,
    /// Cloud density multiplier — affects opacity and shadow depth.
    pub cloud_density: f32,
    /// World-space Y of the cloud slab base.
    pub cloud_height: f32,
    /// Slab thickness in world units.
    pub cloud_thickness: f32,
    /// Wind direction (X, Y, Z). Normalized in-shader; Y component usually 0.
    pub wind_direction: Vec3,
    /// Wind speed in world units per second (scrolls cloud noise).
    pub wind_speed: f32,
}

impl Default for SkyRenderer {
    fn default() -> Self {
        Self {
            active: true,
            priority: 0,
            top_color: Vec3::new(0.5, 0.7, 1.0),
            bottom_color: Vec3::new(0.1, 0.2, 0.4),
            sun_direction: Vec3::new(0.3, 0.7, -0.5),
            sun_color: Vec3::new(1.0, 0.95, 0.85),
            cloud_coverage: 0.45,
            cloud_density: 0.8,
            cloud_height: 80.0,
            cloud_thickness: 60.0,
            wind_direction: Vec3::new(1.0, 0.0, 0.3),
            wind_speed: 2.0,
        }
    }
}

impl Component for SkyRenderer {}

#[cfg(test)]
mod tests;

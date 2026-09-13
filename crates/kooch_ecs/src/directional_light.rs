//! Directional light component.

use glam::Vec3;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Directional light source (e.g. sunlight).
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Rendering")]
pub struct DirectionalLight {
    /// Whether this light contributes to the scene.
    pub active: bool,
    /// Colour, as linear RGB.
    pub color: Vec3,
    /// Illuminance, in LUX — the light landing on a surface facing it square-on.
    pub intensity: f32,
    /// Whether this light casts shadows.
    pub cast_shadows: bool,
    /// Whether this light marches the depth buffer for contact shadows.
    pub contact_shadows: bool,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            active: true,
            color: Vec3::ONE,
            intensity: crate::light_consts::lux::AMBIENT_DAYLIGHT,
            cast_shadows: true,
            contact_shadows: true,
        }
    }
}

impl Component for DirectionalLight {}

#[cfg(test)]
mod tests;

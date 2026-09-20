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
    /// Layers this light lights (#1220). A surface sharing no bit with it takes nothing from this
    /// light, tested per pixel.
    #[reflect(layers)]
    pub layers: u32,
    /// Layers that cast into this light's shadow. Rejected in the shadow view's own cull, so a
    /// caster left out is never rasterised at all.
    #[reflect(layers)]
    pub shadow_layers: u32,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            active: true,
            color: Vec3::ONE,
            intensity: crate::light_consts::lux::AMBIENT_DAYLIGHT,
            cast_shadows: true,
            contact_shadows: true,
            layers: u32::MAX,
            shadow_layers: u32::MAX,
        }
    }
}

impl Component for DirectionalLight {}

#[cfg(test)]
mod tests;

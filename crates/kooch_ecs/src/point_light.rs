//! Point light component.

use glam::Vec3;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Omnidirectional point light source.
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Rendering")]
pub struct PointLight {
    /// Whether this light contributes to the scene.
    pub active: bool,
    /// Colour, as linear RGB.
    pub color: Vec3,
    /// Luminous flux, in LUMENS — the total light emitted in every direction combined.
    pub intensity: f32,
    /// Distance at which the light reaches exactly zero, in world units.
    pub range: f32,
    /// Radius of the emitting sphere, in world units. `0` is a mathematical point.
    pub radius: f32,
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

impl Default for PointLight {
    fn default() -> Self {
        Self {
            active: true,
            color: Vec3::ONE,
            intensity: crate::light_consts::lumens::ROOM_LIGHT_NO_GI,
            range: 10.0,
            radius: 0.0,
            cast_shadows: true,
            contact_shadows: false,
            layers: u32::MAX,
            shadow_layers: u32::MAX,
        }
    }
}

impl Component for PointLight {}

#[cfg(test)]
mod tests;

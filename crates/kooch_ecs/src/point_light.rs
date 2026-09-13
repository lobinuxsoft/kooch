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
        }
    }
}

impl Component for PointLight {}

#[cfg(test)]
mod tests;

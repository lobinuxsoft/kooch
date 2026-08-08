//! Directional light component.
//!
//! Emits parallel light rays in a single direction (sun-like). Uses the
//! forward vector of [`Transform`](crate::Transform)`::rotation` as the
//! light direction. Position is ignored.

use glam::Vec3;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Directional light source (e.g. sunlight).
///
/// # Default
///
/// - `active`: true
/// - `color`: white `(1, 1, 1)`
/// - `intensity`: 10000.0 lux (bright overcast daylight)
/// - `cast_shadows`: true
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Rendering")]
pub struct DirectionalLight {
    /// Whether this light contributes to the scene.
    ///
    /// Unchecking it is not the same as deleting the entity: the light
    /// keeps its rotation and gizmo, and costs nothing.
    pub active: bool,
    /// Colour, as linear RGB.
    ///
    /// Linear, not sRGB — this is the colour the light physically emits,
    /// not a colour picked on a screen.
    pub color: Vec3,
    /// Illuminance, in LUX — the light landing on a surface facing it
    /// square-on.
    ///
    /// Not the same unit as a PointLight's intensity, which is in
    /// lumens, and not comparable to it by eye either. Named values:
    /// an office is 320, an overcast day 1 000, ambient daylight 10 000
    /// (the default), direct sunlight 100 000. See `light_consts::lux`.
    ///
    /// A directional light has no position and no falloff — it stands in
    /// for something so far away that its rays arrive parallel, so the
    /// only thing that matters is which way the entity is pointing.
    pub intensity: f32,
    /// Whether this light casts shadows.
    ///
    /// Cascaded shadow maps (#476). Only the first active directional
    /// light in the scene casts today — #734 is the other half.
    pub cast_shadows: bool,
    /// Whether this light marches the depth buffer for contact shadows.
    ///
    /// Cascades are correct at range and worst at contact: the few
    /// centimetres where an object meets the ground is where its shadow
    /// detaches, and is what makes objects look like they float. A short
    /// screen-space ray fixes that band and nothing else.
    ///
    /// On by default here and off on point and spot lights, because a
    /// scene has one sun and can have fifty lamps: the cost is per light
    /// per pixel, so it is the lamps that need the opt-in and the sun
    /// that needs to be seen.
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
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let l = DirectionalLight::default();
        assert!(l.active);
        assert_eq!(l.color, Vec3::ONE);
        assert_eq!(l.intensity, crate::light_consts::lux::AMBIENT_DAYLIGHT);
        assert!(l.cast_shadows);
        assert!(l.contact_shadows);
    }

    #[test]
    fn reflect_fields() {
        let l = DirectionalLight::default();
        let fields = l.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(
            names,
            &[
                "active",
                "color",
                "intensity",
                "cast_shadows",
                "contact_shadows"
            ]
        );
    }
}

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
    /// Whether this light is active.
    pub active: bool,
    /// Light color (linear RGB).
    pub color: Vec3,
    /// Illuminance in lux.
    pub intensity: f32,
    /// Whether this light casts shadows.
    pub cast_shadows: bool,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            active: true,
            color: Vec3::ONE,
            intensity: 10_000.0,
            cast_shadows: true,
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
        assert_eq!(l.intensity, 10_000.0);
        assert!(l.cast_shadows);
    }

    #[test]
    fn reflect_fields() {
        let l = DirectionalLight::default();
        let fields = l.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(names, &["active", "color", "intensity", "cast_shadows"]);
    }
}

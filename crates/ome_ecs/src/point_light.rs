//! Point light component.
//!
//! Omnidirectional light source that emits from a single point in all
//! directions. Uses [`Transform`](crate::Transform)`::position` as origin;
//! rotation is ignored.

use glam::Vec3;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Omnidirectional point light source.
///
/// # Default
///
/// - `active`: true
/// - `color`: white `(1, 1, 1)`
/// - `intensity`: 800.0 lumens (standard light bulb)
/// - `range`: 10.0
/// - `cast_shadows`: true
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Rendering")]
pub struct PointLight {
    /// Whether this light is active.
    pub active: bool,
    /// Light color (linear RGB).
    pub color: Vec3,
    /// Luminous flux in lumens.
    pub intensity: f32,
    /// Maximum range (attenuation cutoff).
    pub range: f32,
    /// Whether this light casts shadows.
    pub cast_shadows: bool,
}

impl Default for PointLight {
    fn default() -> Self {
        Self {
            active: true,
            color: Vec3::ONE,
            intensity: 800.0,
            range: 10.0,
            cast_shadows: true,
        }
    }
}

impl Component for PointLight {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let l = PointLight::default();
        assert!(l.active);
        assert_eq!(l.color, Vec3::ONE);
        assert_eq!(l.intensity, 800.0);
        assert_eq!(l.range, 10.0);
        assert!(l.cast_shadows);
    }

    #[test]
    fn reflect_fields() {
        let l = PointLight::default();
        let fields = l.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(names, &["active", "color", "intensity", "range", "cast_shadows"]);
    }
}

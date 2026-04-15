//! Spot light component.
//!
//! Cone-shaped light source. Uses [`Transform`](crate::Transform)`::position`
//! as origin and the forward vector of `rotation` as the cone axis. Intensity
//! falls off between `inner_angle` (full) and `outer_angle` (zero).

use glam::Vec3;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Cone-shaped spot light source.
///
/// # Default
///
/// - `active`: true
/// - `color`: white `(1, 1, 1)`
/// - `intensity`: 800.0 lumens
/// - `range`: 10.0
/// - `inner_angle`: 30.0 degrees
/// - `outer_angle`: 45.0 degrees
/// - `cast_shadows`: true
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Rendering")]
pub struct SpotLight {
    /// Whether this light is active.
    pub active: bool,
    /// Light color (linear RGB).
    pub color: Vec3,
    /// Luminous flux in lumens.
    pub intensity: f32,
    /// Maximum range (attenuation cutoff).
    pub range: f32,
    /// Inner cone angle in degrees (full intensity).
    pub inner_angle: f32,
    /// Outer cone angle in degrees (falloff to zero).
    pub outer_angle: f32,
    /// Whether this light casts shadows.
    pub cast_shadows: bool,
}

impl Default for SpotLight {
    fn default() -> Self {
        Self {
            active: true,
            color: Vec3::ONE,
            intensity: 800.0,
            range: 10.0,
            inner_angle: 30.0,
            outer_angle: 45.0,
            cast_shadows: true,
        }
    }
}

impl Component for SpotLight {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let l = SpotLight::default();
        assert!(l.active);
        assert_eq!(l.color, Vec3::ONE);
        assert_eq!(l.intensity, 800.0);
        assert_eq!(l.range, 10.0);
        assert_eq!(l.inner_angle, 30.0);
        assert_eq!(l.outer_angle, 45.0);
        assert!(l.cast_shadows);
    }

    #[test]
    fn reflect_fields() {
        let l = SpotLight::default();
        let fields = l.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(
            names,
            &[
                "active",
                "color",
                "intensity",
                "range",
                "inner_angle",
                "outer_angle",
                "cast_shadows",
            ]
        );
    }
}

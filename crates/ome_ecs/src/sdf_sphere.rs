//! SDF sphere component.
//!
//! Defines a sphere primitive for SDF ray marching and collision.
//! Uses `Transform.position` as center and `Transform.scale` for non-uniform scaling.

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// SDF sphere primitive.
///
/// # Default
///
/// - `visible`: true
/// - `collide`: true
/// - `radius`: 0.5
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "SDF")]
pub struct SdfSphere {
    /// Whether this shape is rendered.
    pub visible: bool,
    /// Whether this shape participates in collision.
    pub collide: bool,
    /// Sphere radius.
    pub radius: f32,
}

impl Default for SdfSphere {
    fn default() -> Self {
        Self {
            visible: true,
            collide: true,
            radius: 0.5,
        }
    }
}

impl Component for SdfSphere {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let s = SdfSphere::default();
        assert!(s.visible);
        assert!(s.collide);
        assert_eq!(s.radius, 0.5);
    }

    #[test]
    fn reflect_fields() {
        let s = SdfSphere::default();
        let fields = s.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(names, &["visible", "collide", "radius"]);
    }
}

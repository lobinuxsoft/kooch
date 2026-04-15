//! SDF capsule component.
//!
//! Defines a capsule (vertical, centered at origin) for SDF ray marching and collision.
//! Uses `Transform.position` as center and `Transform.rotation` for orientation.

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// SDF capsule primitive (two hemispheres connected by a cylinder).
///
/// # Default
///
/// - `visible`: true
/// - `collide`: true
/// - `radius`: 0.25
/// - `half_height`: 0.5
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "SDF")]
pub struct SdfCapsule {
    /// Whether this shape is rendered.
    pub visible: bool,
    /// Whether this shape participates in collision.
    pub collide: bool,
    /// Capsule radius.
    pub radius: f32,
    /// Half-height (distance from center to hemisphere center).
    pub half_height: f32,
}

impl Default for SdfCapsule {
    fn default() -> Self {
        Self {
            visible: true,
            collide: true,
            radius: 0.25,
            half_height: 0.5,
        }
    }
}

impl Component for SdfCapsule {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let c = SdfCapsule::default();
        assert!(c.visible);
        assert!(c.collide);
        assert_eq!(c.radius, 0.25);
        assert_eq!(c.half_height, 0.5);
    }

    #[test]
    fn reflect_fields() {
        let c = SdfCapsule::default();
        let fields = c.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(names, &["visible", "collide", "radius", "half_height"]);
    }
}

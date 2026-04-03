//! SDF cylinder component.
//!
//! Defines a capped cylinder (vertical, centered at origin) for SDF ray marching and collision.
//! Uses `Transform.position` as center and `Transform.rotation` for orientation.

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// SDF capped cylinder primitive.
///
/// # Default
///
/// - `visible`: true
/// - `collide`: true
/// - `radius`: 0.5
/// - `half_height`: 0.5
#[derive(Debug, Clone, Copy, Reflect)]
pub struct SdfCylinder {
    /// Whether this shape is rendered.
    pub visible: bool,
    /// Whether this shape participates in collision.
    pub collide: bool,
    /// Cylinder radius.
    pub radius: f32,
    /// Half-height along the local Y axis.
    pub half_height: f32,
}

impl Default for SdfCylinder {
    fn default() -> Self {
        Self {
            visible: true,
            collide: true,
            radius: 0.5,
            half_height: 0.5,
        }
    }
}

impl Component for SdfCylinder {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let c = SdfCylinder::default();
        assert!(c.visible);
        assert!(c.collide);
        assert_eq!(c.radius, 0.5);
        assert_eq!(c.half_height, 0.5);
    }

    #[test]
    fn reflect_fields() {
        let c = SdfCylinder::default();
        let fields = c.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(names, &["visible", "collide", "radius", "half_height"]);
    }
}

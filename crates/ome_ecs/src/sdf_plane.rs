//! SDF plane component.
//!
//! Defines an infinite plane for SDF ray marching and collision.
//! Uses `Transform.position` as a point on the plane and `Transform.rotation`
//! for the normal direction (local Y+ = plane normal).

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// SDF infinite plane primitive.
///
/// The plane normal is derived from the entity's `Transform.rotation`
/// (local Y+ axis). No geometric parameters needed beyond position and
/// orientation.
///
/// # Default
///
/// - `visible`: true
/// - `collide`: true
#[derive(Debug, Clone, Copy, Reflect)]
pub struct SdfPlane {
    /// Whether this shape is rendered.
    pub visible: bool,
    /// Whether this shape participates in collision.
    pub collide: bool,
}

impl Default for SdfPlane {
    fn default() -> Self {
        Self {
            visible: true,
            collide: true,
        }
    }
}

impl Component for SdfPlane {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let p = SdfPlane::default();
        assert!(p.visible);
        assert!(p.collide);
    }

    #[test]
    fn reflect_fields() {
        let p = SdfPlane::default();
        let fields = p.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(names, &["visible", "collide"]);
    }
}

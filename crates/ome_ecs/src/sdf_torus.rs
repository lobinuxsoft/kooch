//! SDF torus component.
//!
//! Defines a torus primitive for SDF ray marching and collision.
//! Uses `Transform.position` as center and `Transform.rotation` for orientation.

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// SDF torus primitive.
///
/// # Default
///
/// - `visible`: true
/// - `collide`: true
/// - `major_radius`: 0.5 (center of tube to center of torus)
/// - `minor_radius`: 0.15 (tube radius)
#[derive(Debug, Clone, Copy, Reflect)]
pub struct SdfTorus {
    /// Whether this shape is rendered.
    pub visible: bool,
    /// Whether this shape participates in collision.
    pub collide: bool,
    /// Major radius (center of tube to center of torus).
    pub major_radius: f32,
    /// Minor radius (tube radius).
    pub minor_radius: f32,
}

impl Default for SdfTorus {
    fn default() -> Self {
        Self {
            visible: true,
            collide: true,
            major_radius: 0.5,
            minor_radius: 0.15,
        }
    }
}

impl Component for SdfTorus {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let t = SdfTorus::default();
        assert!(t.visible);
        assert!(t.collide);
        assert_eq!(t.major_radius, 0.5);
        assert_eq!(t.minor_radius, 0.15);
    }

    #[test]
    fn reflect_fields() {
        let t = SdfTorus::default();
        let fields = t.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(names, &["visible", "collide", "major_radius", "minor_radius"]);
    }
}

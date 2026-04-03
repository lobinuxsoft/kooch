//! SDF box component.
//!
//! Defines a box primitive for SDF ray marching and collision.
//! Uses `Transform.position` as center and `Transform.rotation` for orientation.

use glam::Vec3;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// SDF box primitive with optional corner rounding.
///
/// # Default
///
/// - `visible`: true
/// - `collide`: true
/// - `size`: (0.5, 0.5, 0.5) half-extents
/// - `rounding`: 0.0
#[derive(Debug, Clone, Copy, Reflect)]
pub struct SdfBox {
    /// Whether this shape is rendered.
    pub visible: bool,
    /// Whether this shape participates in collision.
    pub collide: bool,
    /// Half-extents.
    pub size: Vec3,
    /// Corner rounding radius.
    pub rounding: f32,
}

impl Default for SdfBox {
    fn default() -> Self {
        Self {
            visible: true,
            collide: true,
            size: Vec3::splat(0.5),
            rounding: 0.0,
        }
    }
}

impl Component for SdfBox {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let b = SdfBox::default();
        assert!(b.visible);
        assert!(b.collide);
        assert_eq!(b.size, Vec3::splat(0.5));
        assert_eq!(b.rounding, 0.0);
    }

    #[test]
    fn reflect_fields() {
        let b = SdfBox::default();
        let fields = b.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(names, &["visible", "collide", "size", "rounding"]);
    }
}

//! Perspective camera component for 3D rendering.
//!
//! Defines how the scene is projected using a perspective projection
//! (objects farther away appear smaller). Used by both the SDF ray marcher
//! and the mesh rasterization pass.

use glam::Vec4;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Perspective projection camera.
///
/// Attach to an entity with [`Transform`](crate::Transform) to define
/// a viewpoint. The view direction comes from the transform's rotation.
///
/// # Default
///
/// - `fov`: 60.0 degrees
/// - `near`: 0.1
/// - `far`: 1000.0
/// - `clear_color`: black `(0, 0, 0, 1)`
/// - `priority`: 0
/// - `active`: true
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Camera")]
pub struct PerspectiveCamera {
    /// Whether this camera is active.
    pub active: bool,
    /// Priority for multi-camera rendering (higher = rendered later).
    pub priority: i32,
    /// Field of view in degrees.
    pub fov: f32,
    /// Near clipping plane.
    pub near: f32,
    /// Far clipping plane.
    pub far: f32,
    /// Clear color RGBA (linear).
    pub clear_color: Vec4,
}

impl Default for PerspectiveCamera {
    fn default() -> Self {
        Self {
            active: true,
            priority: 0,
            fov: 60.0,
            near: 0.1,
            far: 1000.0,
            clear_color: Vec4::new(0.0, 0.0, 0.0, 1.0),
        }
    }
}

impl Component for PerspectiveCamera {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let cam = PerspectiveCamera::default();
        assert_eq!(cam.fov, 60.0);
        assert_eq!(cam.near, 0.1);
        assert_eq!(cam.far, 1000.0);
        assert_eq!(cam.clear_color, Vec4::new(0.0, 0.0, 0.0, 1.0));
        assert_eq!(cam.priority, 0);
        assert!(cam.active);
    }

    #[test]
    fn reflect_fields() {
        let cam = PerspectiveCamera::default();
        let fields = cam.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(
            names,
            &["active", "priority", "fov", "near", "far", "clear_color"]
        );
    }
}

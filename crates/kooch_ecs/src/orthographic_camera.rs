//! Orthographic camera component for 2D/isometric rendering.
//!
//! Defines how the scene is projected using an orthographic projection
//! (no perspective foreshortening). Useful for 2D games, UI overlays,
//! and isometric views.

use glam::Vec4;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Orthographic projection camera.
///
/// Attach to an entity with [`Transform`](crate::Transform) to define
/// a viewpoint. The view direction comes from the transform's rotation.
///
/// # Default
///
/// - `size`: 5.0 (half-height in world units)
/// - `near`: 0.1
/// - `far`: 1000.0
/// - `clear_color`: black `(0, 0, 0, 1)`
/// - `priority`: 0
/// - `active`: true
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Camera")]
pub struct OrthographicCamera {
    /// Whether this camera is active.
    pub active: bool,
    /// Priority for multi-camera rendering (higher = rendered later).
    pub priority: i32,
    /// Orthographic size (half-height in world units).
    pub size: f32,
    /// Near clipping plane.
    pub near: f32,
    /// Far clipping plane.
    pub far: f32,
    /// Clear color RGBA (linear).
    pub clear_color: Vec4,
}

impl Default for OrthographicCamera {
    fn default() -> Self {
        Self {
            active: true,
            priority: 0,
            size: 5.0,
            near: 0.1,
            far: 1000.0,
            clear_color: Vec4::new(0.0, 0.0, 0.0, 1.0),
        }
    }
}

impl Component for OrthographicCamera {}

#[cfg(test)]
mod tests;

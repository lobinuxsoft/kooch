//! Orthographic camera component for 2D/isometric rendering.

use glam::Vec4;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Orthographic projection camera.
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
    /// Layers this camera draws (#1219). A renderer sharing no bit with it is not culled in, and
    /// the shadows are not filtered by it — those belong to the light.
    #[reflect(layers)]
    pub culling_mask: u32,
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
            culling_mask: u32::MAX,
        }
    }
}

impl Component for OrthographicCamera {}

#[cfg(test)]
mod tests;

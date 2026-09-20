//! Perspective camera component for 3D rendering.

use glam::Vec4;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Perspective projection camera.
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
    /// Layers this camera draws (#1219). A renderer sharing no bit with it is not culled in, and
    /// the shadows are not filtered by it — those belong to the light.
    #[reflect(layers)]
    pub culling_mask: u32,
    /// Draws over the base camera instead of owning the image (#1221). An overlay brings no sky and
    /// no clear: where it drew nothing, the base stays. Its own `priority` orders it against the
    /// other overlays, and an overlay with no base composes nothing.
    pub overlay: bool,
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
            culling_mask: u32::MAX,
            overlay: false,
        }
    }
}

impl Component for PerspectiveCamera {}

#[cfg(test)]
mod tests;

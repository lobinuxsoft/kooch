//! One camera, as a value a pass can be handed.
//!
//! # Why
//!
//! Four places used to answer "which camera is this frame drawn from" by
//! querying the world for the highest-priority active `PerspectiveCamera`:
//! the game runtime, the editor viewport, the gizmo ray builder and the
//! sky pass. That works while there is exactly one view. With two, the
//! sky pass answered the world's question instead of its caller's, so
//! orbiting the editor camera swung the sky in the Game panel while the
//! meshlet stage — which had already learned to take a view — held still.
//!
//! A pass that resolves its own camera cannot be asked to draw a second
//! view. So the camera becomes a parameter: the caller knows which view
//! it is rendering, and it is the only one that can know.

use glam::{Mat4, Vec3};

use kooch_ecs::PerspectiveCamera;
use kooch_ecs::hierarchy::GlobalTransform;

/// The camera a single view is rendered through.
///
/// Holds the world transform and the lens, not the derived matrices:
/// aspect ratio belongs to the target being drawn into, and two panels
/// showing the same camera at different sizes need different projections
/// from the same `ViewCamera`.
#[derive(Debug, Clone, Copy)]
pub struct ViewCamera {
    /// Camera-to-world. Its translation is the eye position.
    pub world_matrix: Mat4,
    pub fov_y_rad: f32,
    pub near: f32,
    pub far: f32,
}

impl ViewCamera {
    /// Reads one from the components an entity carries, clamping the lens
    /// to values a projection matrix survives.
    pub fn from_components(cam: &PerspectiveCamera, transform: &GlobalTransform) -> Self {
        Self {
            world_matrix: transform.matrix,
            fov_y_rad: cam.fov.to_radians().max(1.0_f32.to_radians()),
            near: cam.near.max(0.001),
            far: cam.far.max(cam.near + 0.001),
        }
    }

    /// World-to-camera.
    pub fn view(&self) -> Mat4 {
        self.world_matrix.inverse()
    }

    /// Reverse-Z perspective for a target of this aspect ratio.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        crate::projection::perspective_rh_reverse_z(
            self.fov_y_rad,
            aspect.max(0.01),
            self.near,
            self.far,
        )
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }

    /// Eye position in world space.
    pub fn position(&self) -> Vec3 {
        self.world_matrix.w_axis.truncate()
    }
}

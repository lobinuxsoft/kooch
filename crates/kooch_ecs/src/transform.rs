//! Transform component — position, rotation, and scale.
//!
//! Fundamental spatial component for all entities that exist in 3D space.

use glam::{DVec3, Mat4, Quat, Vec3};

use crate::component::Component;

// Import the derive macro (re-exported at crate root).
#[allow(unused_imports)]
use crate::Reflect;

/// 3D transform with position, rotation, and scale.
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Core")]
pub struct Transform {
    /// Position relative to the parent, in metres. f64 so a root keeps millimetres a planet radius
    /// out; what the GPU and the solver see is relative to the world origin (#1972).
    pub position: DVec3,
    /// Rotation relative to the parent.
    pub rotation: Quat,
    /// Scale relative to the parent, per axis.
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: DVec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl Component for Transform {}

impl Transform {
    /// Creates a new transform at the given position with default rotation and scale.
    pub fn from_position(position: impl Into<DVec3>) -> Self {
        Self {
            position: position.into(),
            ..Default::default()
        }
    }

    /// Creates a new transform with the given position, rotation, and scale.
    pub fn new(position: impl Into<DVec3>, rotation: Quat, scale: Vec3) -> Self {
        Self {
            position: position.into(),
            rotation,
            scale,
        }
    }

    /// The local matrix (scale × rotation × translation). For a child, whose offset is small; a
    /// root goes through [`Self::matrix_from`].
    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position.as_vec3())
    }

    /// The matrix with the position taken from `origin`, subtracted in f64 before it narrows.
    pub fn matrix_from(&self, origin: DVec3) -> Mat4 {
        let at = (self.position - origin).as_vec3();
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, at)
    }

    /// The position as f32, for a child's small offset.
    pub fn local(&self) -> Vec3 {
        self.position.as_vec3()
    }
}

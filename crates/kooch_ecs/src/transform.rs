//! Transform component — position, rotation, and scale.
//!
//! Fundamental spatial component for all entities that exist in 3D space.

use glam::{Mat4, Quat, Vec3};

use crate::component::Component;

// Import the derive macro (re-exported at crate root).
#[allow(unused_imports)]
use crate::Reflect;

/// 3D transform with position, rotation, and scale.
///
/// # Default
///
/// - `position`: origin `(0, 0, 0)`
/// - `rotation`: identity (no rotation)
/// - `scale`: uniform `(1, 1, 1)`
#[derive(Debug, Clone, Copy, Reflect)]
pub struct Transform {
    /// Position relative to the parent, in world units.
    ///
    /// Relative to the *parent*, not the world — an entity with a parent
    /// at `(10, 0, 0)` and a position of `(1, 0, 0)` sits at `(11, 0, 0)`.
    /// The resolved world position lives on `GlobalTransform`, which the
    /// engine recomputes every frame.
    pub position: Vec3,
    /// Rotation relative to the parent.
    ///
    /// Shown in the Inspector as Euler angles in degrees because nobody
    /// authors a quaternion by hand; stored as a quaternion because Euler
    /// angles gimbal-lock and do not interpolate. Typing 360 and typing 0
    /// therefore give the same rotation, and the field may read back
    /// differently from what was typed.
    ///
    /// An entity's **forward is its local -Z** — that is the direction a
    /// camera looks and a light points.
    pub rotation: Quat,
    /// Scale relative to the parent, per axis.
    ///
    /// `1` is unscaled. Non-uniform scale (different values per axis)
    /// distorts child rotations and skews normals; uniform scale is the
    /// safe case.
    ///
    /// A light's `range` scales with the largest component of this, so
    /// scaling a light entity really does change how far it reaches.
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl Component for Transform {}

impl Transform {
    /// Creates a new transform at the given position with default rotation and scale.
    pub fn from_position(position: Vec3) -> Self {
        Self {
            position,
            ..Default::default()
        }
    }

    /// Creates a new transform with the given position, rotation, and scale.
    pub fn new(position: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self {
            position,
            rotation,
            scale,
        }
    }

    /// Computes the local-space 4x4 matrix (scale × rotation × translation).
    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position)
    }
}

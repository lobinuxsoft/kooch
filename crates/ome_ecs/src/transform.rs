//! Transform component — position, rotation, and scale.
//!
//! Fundamental spatial component for all entities that exist in 3D space.

use glam::{Quat, Vec3};

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
#[derive(Debug, Clone, Copy, Default, Reflect)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
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
}

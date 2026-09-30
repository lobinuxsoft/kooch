//! The bodies that need nothing but the point they follow: Cinemachine's `HardLockToTarget` and
//! `Follow` (#1397).
//!
//! 🔴 A body is **a component**, and which one an entity carries is the only statement of what it
//! does. `VirtualCamera.follow` said the same thing a second time, and the two disagreed: a scene
//! loaded with `follow: Orbital Follow` and no `OrbitalFollow`, the arm was placed by nothing, and
//! the rig said nothing at all.
//!
//! The small ones live together because neither is more than its fields; the ones with arithmetic
//! have files of their own.

use glam::Vec3;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Sits exactly on the target — Cinemachine's `HardLockToTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect)]
#[reflect(category = "Camera")]
pub struct HardLockToTarget;

impl Component for HardLockToTarget {}

/// The target's position plus a fixed offset — Cinemachine's `Follow`.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct Follow {
    /// Added to the target's position, in world space.
    pub offset: Vec3,
}

impl Default for Follow {
    fn default() -> Self {
        Self {
            offset: Vec3::new(0.0, 2.0, 6.0),
        }
    }
}

impl Component for Follow {}

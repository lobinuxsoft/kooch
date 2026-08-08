//! Fly-mode movement for the editor camera.
//!
//! Fly mode is engaged while the user holds RMB inside the viewport.
//! Mouse movement reuses [`apply_yaw_pitch`](super::orbit::apply_yaw_pitch)
//! for look; this module owns only the WASD/QE translation logic.
//!
//! Convention chosen: **W/A/S/D move along the camera's local axes**;
//! **Q/E move along world `±Y`** (deliberately *not* camera-up). Tying
//! vertical motion to world up keeps the FPS feel stable when the
//! camera is pitched, matching Unity / Unreal level-editor cameras.

use glam::{Quat, Vec3};

/// Snapshot of the WASD/QE keys held during a fly-mode tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct FlyKeys {
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
}

impl FlyKeys {
    /// Returns whether any movement key is held.
    pub fn any(self) -> bool {
        self.forward || self.backward || self.left || self.right || self.up || self.down
    }
}

/// World-space displacement to apply to the orbit `focus_point` for a
/// fly-mode tick. Returns `Vec3::ZERO` if no keys are pressed (so
/// callers can early-return without recomputing transforms).
///
/// `dt_seconds` is the frame delta in seconds; `fly_speed` is the
/// controller's configured base speed in world-units per second.
pub fn fly_velocity(keys: FlyKeys, orientation: Quat, fly_speed: f32, dt_seconds: f32) -> Vec3 {
    if !keys.any() || dt_seconds <= 0.0 || fly_speed <= 0.0 {
        return Vec3::ZERO;
    }

    let forward = (orientation * -Vec3::Z).normalize();
    let right = (orientation * Vec3::X).normalize();
    let world_up = Vec3::Y;

    let mut direction = Vec3::ZERO;
    if keys.forward {
        direction += forward;
    }
    if keys.backward {
        direction -= forward;
    }
    if keys.right {
        direction += right;
    }
    if keys.left {
        direction -= right;
    }
    if keys.up {
        direction += world_up;
    }
    if keys.down {
        direction -= world_up;
    }

    if direction.length_squared() < 1e-8 {
        // Opposing keys cancelled out.
        return Vec3::ZERO;
    }

    direction.normalize() * fly_speed * dt_seconds
}

#[cfg(test)]
mod tests;

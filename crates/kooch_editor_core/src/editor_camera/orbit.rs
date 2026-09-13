//! Orbit and position math for the editor camera.

use glam::{Quat, Vec3};

/// Applies yaw-then-pitch rotation to a camera orientation quaternion.
pub fn apply_yaw_pitch(orientation: Quat, yaw_delta: f32, pitch_delta: f32) -> Quat {
    let yaw_quat = Quat::from_axis_angle(Vec3::Y, yaw_delta);
    let after_yaw = yaw_quat * orientation;
    let right = (after_yaw * Vec3::X).normalize();
    let pitch_quat = Quat::from_axis_angle(right, pitch_delta);
    (pitch_quat * after_yaw).normalize()
}

/// Computes the world-space camera position from orbit state.
pub fn camera_position(focus_point: Vec3, orientation: Quat, distance: f32) -> Vec3 {
    let forward = orientation * -Vec3::Z;
    focus_point - forward * distance
}

/// Rotates the camera around its own position (FPS-style look) instead of around `focus_point`, and
/// returns the re-anchored focus point.
pub fn fly_look_pivot_camera(
    position: Vec3,
    orientation: Quat,
    distance: f32,
    yaw_delta: f32,
    pitch_delta: f32,
) -> (Quat, Vec3) {
    let new_orientation = apply_yaw_pitch(orientation, yaw_delta, pitch_delta);
    let forward = new_orientation * -Vec3::Z;
    let new_focus = position + forward * distance;
    (new_orientation, new_focus)
}

#[cfg(test)]
mod tests;

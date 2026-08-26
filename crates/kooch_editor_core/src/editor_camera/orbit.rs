//! Orbit and position math for the editor camera.
//!
//! All functions are pure: they take the current state plus deltas and
//! return new values. No egui, no resources, no side effects — that
//! makes them straightforward to unit test in isolation.

use glam::{Quat, Vec3};

/// Applies yaw-then-pitch rotation to a camera orientation quaternion.
///
/// Yaw rotates around the world `+Y` axis; pitch rotates around the
/// camera's local `+X` axis (the right vector after yaw is applied).
/// Composition is `pitch * yaw * orientation`, i.e. the camera is first
/// yawed in world space and then pitched in its own frame.
///
/// The function does **not** clamp pitch — the camera can roll over the
/// top. We accept this for v1 (math stays pure quaternion); a future
/// enhancement may clamp pitch by detecting up-vector inversion.
pub fn apply_yaw_pitch(orientation: Quat, yaw_delta: f32, pitch_delta: f32) -> Quat {
    let yaw_quat = Quat::from_axis_angle(Vec3::Y, yaw_delta);
    let after_yaw = yaw_quat * orientation;
    let right = (after_yaw * Vec3::X).normalize();
    let pitch_quat = Quat::from_axis_angle(right, pitch_delta);
    (pitch_quat * after_yaw).normalize()
}

/// Computes the world-space camera position from orbit state.
///
/// In the right-handed view-space convention the camera looks down `-Z`,
/// so the world-space forward is `orientation * -Z`. The camera sits at
/// `distance` units behind the focus point along that forward axis.
pub fn camera_position(focus_point: Vec3, orientation: Quat, distance: f32) -> Vec3 {
    let forward = orientation * -Vec3::Z;
    focus_point - forward * distance
}

/// Rotates the camera around its own position (FPS-style look) instead
/// of around `focus_point`, and returns the re-anchored focus point.
///
/// Internally this just calls [`apply_yaw_pitch`] for the rotation,
/// then re-derives `focus_point` so it sits `distance` units in front
/// of the (unchanged) camera position. This keeps the orbit pivot
/// available for whatever the user looks at when fly mode ends.
///
/// Returns `(new_orientation, new_focus_point)`.
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

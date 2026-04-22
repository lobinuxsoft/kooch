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

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: Vec3, b: Vec3, eps: f32) -> bool {
        (a - b).length() < eps
    }

    #[test]
    fn zero_deltas_preserve_orientation() {
        let orientation = Quat::from_rotation_y(0.5);
        let result = apply_yaw_pitch(orientation, 0.0, 0.0);
        assert!((orientation * Vec3::Z - result * Vec3::Z).length() < 1e-5);
    }

    #[test]
    fn yaw_rotates_around_world_up() {
        // Identity orientation looks down -Z (forward = -Z, right = +X).
        let orientation = Quat::IDENTITY;
        // Quarter-turn yaw left.
        let after = apply_yaw_pitch(orientation, std::f32::consts::FRAC_PI_2, 0.0);
        // Forward should now be -X (looking down +X axis turns into looking down -X).
        let forward = after * -Vec3::Z;
        assert!(approx_eq(forward.normalize(), Vec3::NEG_X, 1e-5));
    }

    #[test]
    fn positive_pitch_tilts_camera_up() {
        // Identity orientation: forward = -Z, right = +X.
        // Rotation by +90° around +X (right-hand rule) carries -Z to +Y,
        // so the camera ends up looking straight up.
        let after = apply_yaw_pitch(Quat::IDENTITY, 0.0, std::f32::consts::FRAC_PI_2);
        let forward = (after * -Vec3::Z).normalize();
        assert!(
            approx_eq(forward, Vec3::Y, 1e-5),
            "expected forward +Y (camera looking up), got {forward:?}",
        );
    }

    #[test]
    fn negative_pitch_tilts_camera_down() {
        let after = apply_yaw_pitch(Quat::IDENTITY, 0.0, -std::f32::consts::FRAC_PI_2);
        let forward = (after * -Vec3::Z).normalize();
        assert!(
            approx_eq(forward, Vec3::NEG_Y, 1e-5),
            "expected forward -Y (camera looking down), got {forward:?}",
        );
    }

    #[test]
    fn camera_position_is_distance_away_from_focus() {
        let focus = Vec3::new(1.0, 2.0, 3.0);
        let orientation = Quat::IDENTITY;
        let distance = 5.0;
        let pos = camera_position(focus, orientation, distance);
        assert!(((pos - focus).length() - distance).abs() < 1e-5);
    }

    #[test]
    fn camera_position_on_minus_z_axis_for_identity_orientation() {
        // Identity orientation looks down -Z; camera should be at +Z.
        let pos = camera_position(Vec3::ZERO, Quat::IDENTITY, 10.0);
        assert!(approx_eq(pos, Vec3::new(0.0, 0.0, 10.0), 1e-5));
    }
}

use super::*;

#[test]
fn zero_drag_yields_zero_delta() {
    let delta = pan_delta(0.0, 0.0, 0.01, Quat::IDENTITY);
    assert_eq!(delta, Vec3::ZERO);
}

#[test]
fn drag_right_pushes_focus_left() {
    // Identity orientation: right = +X. Drag +100 px right with sens=0.01
    // → focus moves -1.0 along +X.
    let delta = pan_delta(100.0, 0.0, 0.01, Quat::IDENTITY);
    assert!((delta - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-5);
}

#[test]
fn drag_down_pushes_focus_up() {
    // egui +y is down on screen → focus moves +Y in world.
    let delta = pan_delta(0.0, 100.0, 0.01, Quat::IDENTITY);
    assert!((delta - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-5);
}

#[test]
fn pan_uses_camera_basis_after_yaw() {
    // 90° yaw left: camera-right is now -Z (was +X).
    let orientation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let delta = pan_delta(100.0, 0.0, 0.01, orientation);
    // Drag right → focus moves opposite of camera-right = +Z.
    assert!(
        (delta - Vec3::new(0.0, 0.0, 1.0)).length() < 1e-4,
        "got {delta:?}"
    );
}

#[test]
fn zoom_in_decreases_distance() {
    let after = apply_zoom(10.0, 1.0, 1.1);
    assert!(after < 10.0);
    assert!((after - (10.0 / 1.1)).abs() < 1e-5);
}

#[test]
fn zoom_out_increases_distance() {
    let after = apply_zoom(10.0, -1.0, 1.1);
    assert!(after > 10.0);
    assert!((after - (10.0 * 1.1)).abs() < 1e-5);
}

#[test]
fn zoom_zero_lines_is_no_op() {
    assert_eq!(apply_zoom(7.5, 0.0, 1.1), 7.5);
}

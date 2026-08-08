use super::*;

#[test]
fn initial_transform_places_camera_at_default_eye() {
    let controller = EditorCameraController::default();
    let t = initial_transform(&controller);
    let delta = (t.position - DEFAULT_EYE).length();
    assert!(
        delta < 1e-4,
        "expected position {DEFAULT_EYE:?}, got {:?}",
        t.position
    );
}

#[test]
fn initial_transform_looks_at_focus_point() {
    let controller = EditorCameraController::default();
    let t = initial_transform(&controller);
    // Camera-forward in glam right-handed view space is -Z, so the
    // world-space forward direction is `rotation * -Z`.
    let forward = (t.rotation * -Vec3::Z).normalize();
    let expected = (controller.focus_point - t.position).normalize();
    let dot = forward.dot(expected);
    assert!(
        dot > 0.999,
        "forward {forward:?} should point at focus, dot={dot}"
    );
}

#[test]
fn editor_camera_priority_is_above_default() {
    // PerspectiveCamera default priority is 0; editor must override.
    assert!(EDITOR_CAMERA_PRIORITY > 0);
}

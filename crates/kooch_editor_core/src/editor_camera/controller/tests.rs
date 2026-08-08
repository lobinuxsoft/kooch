use super::*;

#[test]
fn defaults_are_sane() {
    let c = EditorCameraController::default();
    assert_eq!(c.focus_point, Vec3::ZERO);
    assert!(c.min_distance < c.distance);
    assert!(c.distance < c.max_distance);
}

#[test]
fn clamp_distance_enforces_bounds() {
    let mut c = EditorCameraController::default();
    c.distance = 50_000.0;
    c.clamp_distance();
    assert_eq!(c.distance, c.max_distance);

    c.distance = -1.0;
    c.clamp_distance();
    assert_eq!(c.distance, c.min_distance);
}

#[test]
fn pan_sensitivity_scales_with_distance() {
    let mut c = EditorCameraController::default();
    c.distance = 1.0;
    let near = c.effective_pan_sensitivity();
    c.distance = 100.0;
    let far = c.effective_pan_sensitivity();
    assert!(far > near, "pan should be slower up-close, faster far-away");
}

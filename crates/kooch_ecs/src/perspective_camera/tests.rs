use super::*;
use crate::reflect::Reflect;

#[test]
fn default_values() {
    let cam = PerspectiveCamera::default();
    assert_eq!(cam.fov, 60.0);
    assert_eq!(cam.near, 0.1);
    assert_eq!(cam.far, 1000.0);
    assert_eq!(cam.clear_color, Vec4::new(0.0, 0.0, 0.0, 1.0));
    assert_eq!(cam.priority, 0);
    assert!(cam.active);
}

#[test]
fn reflect_fields() {
    let cam = PerspectiveCamera::default();
    let fields = cam.reflect_fields();
    let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
    assert_eq!(
        names,
        &["active", "priority", "fov", "near", "far", "clear_color"]
    );
}

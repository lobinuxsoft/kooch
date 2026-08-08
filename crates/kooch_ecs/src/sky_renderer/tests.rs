use super::*;
use crate::reflect::Reflect;

#[test]
fn default_values() {
    let s = SkyRenderer::default();
    assert!(s.active);
    assert_eq!(s.priority, 0);
    assert_eq!(s.top_color, Vec3::new(0.5, 0.7, 1.0));
    assert_eq!(s.bottom_color, Vec3::new(0.1, 0.2, 0.4));
    assert!(s.cloud_coverage > 0.0 && s.cloud_coverage < 1.0);
}

#[test]
fn reflect_fields() {
    let s = SkyRenderer::default();
    let fields = s.reflect_fields();
    let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
    assert_eq!(
        names,
        &[
            "active",
            "priority",
            "top_color",
            "bottom_color",
            "sun_direction",
            "sun_color",
            "cloud_coverage",
            "cloud_density",
            "cloud_height",
            "cloud_thickness",
            "wind_direction",
            "wind_speed",
        ]
    );
}

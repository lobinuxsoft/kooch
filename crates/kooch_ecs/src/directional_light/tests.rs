use super::*;
use crate::reflect::Reflect;

#[test]
fn default_values() {
    let l = DirectionalLight::default();
    assert!(l.active);
    assert_eq!(l.color, Vec3::ONE);
    assert_eq!(l.intensity, crate::light_consts::lux::AMBIENT_DAYLIGHT);
    assert!(l.cast_shadows);
    assert!(l.contact_shadows);
}

#[test]
fn reflect_fields() {
    let l = DirectionalLight::default();
    let fields = l.reflect_fields();
    let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
    assert_eq!(
        names,
        &[
            "active",
            "color",
            "intensity",
            "cast_shadows",
            "contact_shadows"
        ]
    );
}

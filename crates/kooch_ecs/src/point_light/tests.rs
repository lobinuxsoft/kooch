use super::*;
use crate::reflect::Reflect;

#[test]
fn default_values() {
    let l = PointLight::default();
    assert!(l.active);
    assert_eq!(l.color, Vec3::ONE);
    // Deliberately far above a real fixture: direct lighting only,
    // so the bounces that make a real room bright are missing. Goes
    // back to a real bulb the day #450 lands.
    assert_eq!(l.intensity, crate::light_consts::lumens::ROOM_LIGHT_NO_GI);
    assert_eq!(l.range, 10.0);
    // A point, the way every light in the engine was before #776 —
    // so an existing scene renders unchanged when it loads.
    assert_eq!(l.radius, 0.0);
    assert!(l.cast_shadows);
    assert!(!l.contact_shadows);
}

#[test]
fn reflect_fields() {
    let l = PointLight::default();
    let fields = l.reflect_fields();
    let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
    assert_eq!(
        names,
        &[
            "active",
            "color",
            "intensity",
            "range",
            "radius",
            "cast_shadows",
            "contact_shadows"
        ]
    );
}

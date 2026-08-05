//! #737 — a field's doc comment reaches the Inspector.
//!
//! The information was always there. It sat in the source, where nobody
//! authoring a scene reads it, while the Inspector showed a name and a
//! number. These tests pin the harvest, because the failure mode is
//! silent: a derive that stops collecting docs produces empty tooltips,
//! and an empty tooltip looks exactly like a field nobody documented.

use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::point_light::PointLight;
use kooch_ecs::reflect::Reflect;
use kooch_ecs::spot_light::SpotLight;
use kooch_ecs::transform::Transform;

fn doc_of<T: Reflect>(value: &T, field: &str) -> &'static str {
    value
        .reflect_fields()
        .iter()
        .find(|m| m.name == field)
        .unwrap_or_else(|| panic!("no field named {field}"))
        .doc
}

#[test]
fn the_derive_harvests_doc_comments() {
    let doc = doc_of(&Transform::default(), "position");
    assert!(
        !doc.is_empty(),
        "Transform::position has a doc comment in the source and none in its FieldMeta",
    );
}

/// The case that prompted the whole issue. Two components, two fields
/// both called `intensity`, two different units, and an Inspector that
/// presented them identically.
#[test]
fn the_two_intensities_say_which_unit_they_are_in() {
    let directional = doc_of(&DirectionalLight::default(), "intensity");
    let point = doc_of(&PointLight::default(), "intensity");

    assert!(
        directional.contains("LUX"),
        "a DirectionalLight's intensity is in lux and does not say so: {directional:?}",
    );
    assert!(
        point.contains("LUMENS"),
        "a PointLight's intensity is in lumens and does not say so: {point:?}",
    );
    assert_ne!(
        directional, point,
        "the two intensities are different units and must not read the same",
    );
}

/// `gizmos/lights.rs` chose half-angles when it drew the cone and wrote
/// down that the lighting work would either honour the convention or
/// draw a cone half the width it lights. The tooltip is where an author
/// finds out which one it was.
#[test]
fn the_spot_cone_angles_say_they_are_half_angles() {
    let inner = doc_of(&SpotLight::default(), "inner_angle");
    let outer = doc_of(&SpotLight::default(), "outer_angle");
    assert!(inner.contains("HALF-angle"), "{inner:?}");
    assert!(outer.contains("HALF-angle"), "{outer:?}");
}

/// Every light field carries an explanation, since "which of these do I
/// touch" is the question the Inspector is failing to answer.
#[test]
fn no_light_field_is_left_unexplained() {
    let mut missing = Vec::new();
    for (component, fields) in [
        (
            "DirectionalLight",
            DirectionalLight::default().reflect_fields(),
        ),
        ("PointLight", PointLight::default().reflect_fields()),
        ("SpotLight", SpotLight::default().reflect_fields()),
    ] {
        for meta in fields {
            if meta.doc.trim().is_empty() {
                missing.push(format!("{component}::{}", meta.name));
            }
        }
    }
    assert!(missing.is_empty(), "fields with no tooltip: {missing:?}");
}

/// The leading space Rust inserts after `///` would indent every line of
/// every tooltip.
#[test]
fn the_harvested_text_is_prose_and_not_source() {
    let doc = doc_of(&PointLight::default(), "range");
    assert!(!doc.starts_with(' '), "leading space survived: {doc:?}");
    assert!(!doc.contains("///"), "comment markers survived: {doc:?}");
    assert!(
        !doc.ends_with('\n'),
        "trailing blank line survived, which renders as empty space: {doc:?}",
    );
}

/// A field with no doc comment yields an empty string, so the Inspector
/// can tell "nothing to say" from "here is nothing".
#[test]
fn an_undocumented_field_yields_empty_rather_than_whitespace() {
    #[derive(Debug, Clone, Copy, Default, kooch_ecs::Reflect)]
    struct Undocumented {
        value: f32,
    }
    assert_eq!(doc_of(&Undocumented::default(), "value"), "");
}

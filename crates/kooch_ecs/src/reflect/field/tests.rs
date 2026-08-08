use super::*;

const SPHERE: i64 = 0;
const CUBOID: i64 = 1;
const CAPSULE: i64 = 2;

static RADIUS_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[SPHERE, CAPSULE],
};

#[test]
fn a_condition_is_met_only_by_its_listed_values() {
    assert!(RADIUS_WHEN.is_met(Some(SPHERE)));
    assert!(RADIUS_WHEN.is_met(Some(CAPSULE)));
    assert!(!RADIUS_WHEN.is_met(Some(CUBOID)));
}

/// A condition naming a field the component does not have reads as
/// met. A typo in an attribute should look like a mistake, not like a
/// field that silently vanished — the field the author annotated is
/// still the field they wanted to see.
#[test]
fn a_missing_discriminant_shows_the_field() {
    assert!(RADIUS_WHEN.is_met(None));
}

/// An empty value list hides the field for every discriminant. Not a
/// useful annotation, but it must not read as "always shown", or a
/// mistake there would be invisible.
#[test]
fn an_empty_condition_hides_the_field() {
    static NEVER: FieldCondition = FieldCondition {
        field: "shape",
        values: &[],
    };
    assert!(!NEVER.is_met(Some(SPHERE)));
    assert!(NEVER.is_met(None), "a missing discriminant still shows");
}

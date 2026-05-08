use super::common::Health;
use crate::reflect::{FieldKind, Reflect, ReflectError, ReflectValue};

// -- Reflect trait tests --------------------------------------------------

#[test]
fn reflect_fields_returns_metadata() {
    let h = Health { hp: 50, max_hp: 100 };
    let fields = h.reflect_fields();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "hp");
    assert_eq!(fields[0].kind, FieldKind::U32);
    assert_eq!(fields[1].name, "max_hp");
}

#[test]
fn reflect_get_returns_values() {
    let h = Health { hp: 42, max_hp: 100 };
    assert_eq!(h.reflect_get("hp"), Some(ReflectValue::U32(42)));
    assert_eq!(h.reflect_get("max_hp"), Some(ReflectValue::U32(100)));
    assert_eq!(h.reflect_get("nonexistent"), None);
}

#[test]
fn reflect_set_modifies_values() {
    let mut h = Health { hp: 50, max_hp: 100 };
    h.reflect_set("hp", ReflectValue::U32(75)).unwrap();
    assert_eq!(h.hp, 75);
}

#[test]
fn reflect_set_type_mismatch() {
    let mut h = Health { hp: 50, max_hp: 100 };
    let err = h.reflect_set("hp", ReflectValue::F32(1.0)).unwrap_err();
    assert_eq!(
        err,
        ReflectError::TypeMismatch {
            field: "hp".into(),
            expected: FieldKind::U32,
            got: FieldKind::F32,
        }
    );
}

#[test]
fn reflect_set_field_not_found() {
    let mut h = Health { hp: 50, max_hp: 100 };
    let err = h.reflect_set("nope", ReflectValue::U32(1)).unwrap_err();
    assert_eq!(err, ReflectError::FieldNotFound("nope".into()));
}

#[test]
fn reflect_default_creates_instance() {
    let h = Health::reflect_default();
    assert_eq!(h.hp, 100);
    assert_eq!(h.max_hp, 100);
}

#[test]
fn reflect_value_kind() {
    assert_eq!(ReflectValue::F32(1.0).kind(), FieldKind::F32);
    assert_eq!(ReflectValue::U32(1).kind(), FieldKind::U32);
    assert_eq!(ReflectValue::Bool(true).kind(), FieldKind::Bool);
    assert_eq!(
        ReflectValue::Vec3(glam::Vec3::ZERO).kind(),
        FieldKind::Vec3
    );
}

#[test]
fn reflect_value_display() {
    assert_eq!(format!("{}", ReflectValue::U32(42)), "42");
    assert_eq!(format!("{}", ReflectValue::F32(3.14)), "3.14");
    assert_eq!(format!("{}", ReflectValue::Bool(true)), "true");
    assert_eq!(
        format!("{}", ReflectValue::Vec3(glam::Vec3::new(1.0, 2.0, 3.0))),
        "(1, 2, 3)"
    );
}

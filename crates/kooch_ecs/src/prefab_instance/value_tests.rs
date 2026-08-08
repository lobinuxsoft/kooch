use super::*;

fn at(field: &str) -> OverrideAddress {
    OverrideAddress {
        entity: 0,
        component: "test::Health".into(),
        field: field.into(),
    }
}

/// The whole reason values are carried: a scene stores the instance as
/// a reference plus this list, so an override that recorded only
/// *which* field changed would be a change that vanished on save.
#[test]
fn a_value_survives_the_round_trip() {
    let mut instance = PrefabInstance::new(Guid::new_v4());
    instance.mark(at("hp"), Some(ReflectValue::U32(37)));
    assert_eq!(instance.value_of(&at("hp")), Some(ReflectValue::U32(37)));
}

/// Changing the same field twice is one override, and the second value
/// is the one that survives — otherwise a drag would leave a trail of
/// stale values, and whichever the loader read last would win.
#[test]
fn re_marking_replaces_the_value() {
    let mut instance = PrefabInstance::default();
    instance.mark(at("hp"), Some(ReflectValue::U32(1)));
    instance.mark(at("hp"), Some(ReflectValue::U32(2)));
    assert_eq!(instance.addresses().len(), 1);
    assert_eq!(instance.value_of(&at("hp")), Some(ReflectValue::U32(2)));
}

/// A removal is the other kind of decision and has nothing to carry.
#[test]
fn a_presence_record_carries_no_value() {
    let mut instance = PrefabInstance::default();
    instance.mark(at(WHOLE_COMPONENT), None);
    assert!(instance.owns_component(0, "test::Health"));
    assert_eq!(instance.value_of(&at(WHOLE_COMPONENT)), None);
}

/// The old separator was `;`, which a record could not contain while
/// it was only an address. Now it ends in a serialised value, and a
/// string field holding a semicolon would have split one record into
/// two.
#[test]
fn a_value_containing_punctuation_does_not_split_the_record() {
    let mut instance = PrefabInstance::default();
    let awkward = ReflectValue::String("a;b\u{1f}c".into());
    instance.mark(at("name"), Some(awkward.clone()));
    instance.mark(at("hp"), Some(ReflectValue::U32(1)));

    assert_eq!(instance.addresses().len(), 2, "the value split the record");
    assert_eq!(instance.value_of(&at("name")), Some(awkward));
}

/// Every `ReflectValue` a component can hold has to survive, not just
/// the scalars — a Transform override is three of these.
#[test]
fn the_shapes_a_transform_override_needs_all_round_trip() {
    let mut instance = PrefabInstance::default();
    for (field, value) in [
        (
            "position",
            ReflectValue::Vec3(glam::Vec3::new(1.0, -2.5, 3.0)),
        ),
        ("rotation", ReflectValue::Quat(glam::Quat::IDENTITY)),
        ("visible", ReflectValue::Bool(true)),
    ] {
        instance.mark(at(field), Some(value.clone()));
        assert_eq!(instance.value_of(&at(field)), Some(value), "{field}");
    }
}

/// A hand-edited file costs the record it corrupted, not the set.
#[test]
fn a_value_that_will_not_parse_costs_only_its_own_record() {
    let mut instance = PrefabInstance::new(Guid::new_v4());
    instance.mark(at("hp"), Some(ReflectValue::U32(5)));
    instance
        .overrides
        .push_str("\u{1e}0\u{1f}test::Health\u{1f}max_hp\u{1f}not-ron");

    assert_eq!(instance.addresses().len(), 1);
    assert!(instance.source.is_some());
}

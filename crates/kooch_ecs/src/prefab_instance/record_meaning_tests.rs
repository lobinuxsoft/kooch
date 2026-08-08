use super::*;

/// What a record means is decided by its field, not by whether a value
/// came with it.
///
/// A removal is the record with no field. A *field* record that
/// arrived without one — hand-edited, or written by a build from
/// before values were carried — is one nothing can apply. Reading it
/// as a removal deletes the component instead, which is how adding a
/// component to a prefab could come back with it missing.
#[test]
fn a_field_record_without_a_value_is_not_a_removal() {
    let mut instance = PrefabInstance::new(Guid::new_v4());
    instance.mark(
        OverrideAddress {
            entity: 0,
            component: "test::Health".into(),
            field: "hp".into(),
        },
        None,
    );

    assert!(
        !instance.owns_component(0, "test::Health"),
        "a valueless field record claimed the whole component",
    );
}

/// And the record that *is* a removal still reads as one.
#[test]
fn the_record_with_no_field_is_a_removal() {
    let mut instance = PrefabInstance::new(Guid::new_v4());
    instance.mark(
        OverrideAddress {
            entity: 0,
            component: "test::Health".into(),
            field: WHOLE_COMPONENT.into(),
        },
        None,
    );
    assert!(instance.owns_component(0, "test::Health"));
}

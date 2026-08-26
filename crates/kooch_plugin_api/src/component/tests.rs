use super::*;

#[test]
fn a_schema_is_built_fluently() {
    let schema = ComponentSchema::new("my_game::Health")
        .with_field("current", FieldKind::U32)
        .with_field("regen", FieldKind::F32);

    assert_eq!(schema.type_name, "my_game::Health");
    assert_eq!(schema.fields.len(), 2);
    assert_eq!(schema.fields[1].kind, FieldKind::F32);
}

/// A marker component is a real thing, not a half-built schema.
#[test]
fn a_marker_has_no_fields() {
    assert!(ComponentSchema::new("my_game::Player").fields.is_empty());
}

/// `ALL` exists so the engine can prove it maps every kind. If a
/// variant is added without listing it here, the mapping would
/// silently skip it.
#[test]
fn all_lists_every_kind() {
    // Adding a variant without extending ALL leaves this stale, and
    // the engine-side parity test then fails loudly.
    assert_eq!(FieldKind::ALL.len(), 20);
    assert_eq!(FieldKind::ALL[0], FieldKind::F32);
    assert_eq!(FieldKind::ALL[19], FieldKind::Nested);

    let mut seen = std::collections::HashSet::new();
    for kind in FieldKind::ALL {
        assert!(seen.insert(*kind), "{kind:?} listed twice in ALL");
    }
}

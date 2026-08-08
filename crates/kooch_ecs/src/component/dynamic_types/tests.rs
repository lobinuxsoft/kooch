use super::*;

fn health(source: &str) -> DynamicType {
    DynamicType {
        type_name: "my_game::Health".into(),
        fields: vec![
            DynamicField {
                name: "current".into(),
                kind: FieldKind::U32,
            },
            DynamicField {
                name: "max".into(),
                kind: FieldKind::U32,
            },
        ],
        defaults: Vec::new(),
        source: source.into(),
    }
}

#[test]
fn a_registered_type_is_found_by_name() {
    let mut registry = DynamicTypeRegistry::new();
    assert!(registry.register(health("game")).is_ok());

    let found = registry.get("my_game::Health").expect("registered");
    assert_eq!(found.fields.len(), 2);
    assert_eq!(found.fields[0].kind, FieldKind::U32);
    assert!(registry.contains("my_game::Health"));
    assert_eq!(registry.len(), 1);
}

/// A reload re-registers the same types. If that failed, reloading
/// would be impossible.
#[test]
fn the_same_source_may_register_twice() {
    let mut registry = DynamicTypeRegistry::new();
    registry.register(health("game")).unwrap();

    assert!(registry.register(health("game")).is_ok());
    assert_eq!(registry.len(), 1, "must not duplicate");
}

/// After a reload the new definition wins — the plugin author may
/// have added a field, and the editor must show it.
#[test]
fn a_changed_schema_from_the_same_source_replaces_the_old() {
    let mut registry = DynamicTypeRegistry::new();
    registry.register(health("game")).unwrap();

    let mut grown = health("game");
    grown.fields.push(DynamicField {
        name: "regen".into(),
        kind: FieldKind::F32,
    });
    registry.register(grown).unwrap();

    assert_eq!(registry.get("my_game::Health").unwrap().fields.len(), 3);
}

/// Two plugins claiming one name is a real collision, and the
/// registry cannot pick a winner.
#[test]
fn a_different_source_is_refused_and_names_the_owner() {
    let mut registry = DynamicTypeRegistry::new();
    registry.register(health("game")).unwrap();

    let err = registry.register(health("mod")).unwrap_err();
    assert_eq!(err, "game");
    assert_eq!(registry.get("my_game::Health").unwrap().source, "game");
}

#[test]
fn unloading_a_source_drops_only_its_types() {
    let mut registry = DynamicTypeRegistry::new();
    registry.register(health("game")).unwrap();
    registry
        .register(DynamicType {
            type_name: "mod::Extra".into(),
            fields: Vec::new(),
            defaults: Vec::new(),
            source: "mod".into(),
        })
        .unwrap();

    assert_eq!(registry.remove_source("game"), 1);
    assert!(!registry.contains("my_game::Health"));
    assert!(registry.contains("mod::Extra"));
}

#[test]
fn a_marker_type_registers() {
    let mut registry = DynamicTypeRegistry::new();
    registry
        .register(DynamicType {
            type_name: "my_game::Player".into(),
            fields: Vec::new(),
            defaults: Vec::new(),
            source: "game".into(),
        })
        .unwrap();

    assert!(registry.get("my_game::Player").unwrap().fields.is_empty());
}

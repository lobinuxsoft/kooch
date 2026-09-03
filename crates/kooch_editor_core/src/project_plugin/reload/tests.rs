use kooch_ecs::component::{DynamicField, DynamicType, DynamicTypeRegistry};
use kooch_ecs::reflect::FieldKind;

use super::Reloaded;

fn registry(types: &[(&str, &[(&str, FieldKind)])]) -> DynamicTypeRegistry {
    let mut registry = DynamicTypeRegistry::new();
    for (name, fields) in types {
        registry
            .register(DynamicType {
                type_name: (*name).to_owned(),
                fields: fields
                    .iter()
                    .map(|(n, k)| DynamicField {
                        name: (*n).to_owned(),
                        kind: *k,
                    })
                    .collect(),
                defaults: Vec::new(),
                source: "game".to_owned(),
            })
            .expect("register");
    }
    registry
}

/// The reported case: a field added to a component that already existed.
#[test]
fn a_new_field_is_named() {
    let before = registry(&[("game::PlayerInput", &[("movement", FieldKind::AssetRef)])]);
    let after = registry(&[(
        "game::PlayerInput",
        &[
            ("movement", FieldKind::AssetRef),
            ("sprint", FieldKind::AssetRef),
        ],
    )]);

    let report = Reloaded::between(&before, &after);

    assert!(!report.is_quiet());
    assert_eq!(report.changed.len(), 1);
    assert_eq!(report.changed[0].added, vec!["sprint".to_owned()]);
    assert!(report.changed[0].removed.is_empty());
}

/// 🔴 The loud one. A dropped field takes its value off every entity
/// carrying the component, and nothing else in the engine says so.
#[test]
fn a_dropped_field_is_named() {
    let before = registry(&[(
        "game::Health",
        &[("hp", FieldKind::F32), ("armour", FieldKind::F32)],
    )]);
    let after = registry(&[("game::Health", &[("hp", FieldKind::F32)])]);

    let report = Reloaded::between(&before, &after);

    assert_eq!(report.changed[0].removed, vec!["armour".to_owned()]);
}

/// Same name, different kind: the value cannot be carried across, and
/// reporting it as "unchanged" would be a lie the Inspector then acts on.
#[test]
fn a_retyped_field_is_not_a_survivor() {
    let before = registry(&[("game::Spin", &[("speed", FieldKind::F32)])]);
    let after = registry(&[("game::Spin", &[("speed", FieldKind::Vec3)])]);

    let report = Reloaded::between(&before, &after);

    assert_eq!(report.changed[0].retyped, vec!["speed".to_owned()]);
    assert!(report.changed[0].added.is_empty(), "counted twice");
    assert!(report.changed[0].removed.is_empty(), "counted twice");
}

/// A rename is a loss and a gain, never a change: the two schemas share
/// no identity, and carrying values across would put them in a type that
/// never held them.
#[test]
fn a_rename_is_a_loss_and_a_gain() {
    let before = registry(&[("game::Health", &[("hp", FieldKind::F32)])]);
    let after = registry(&[("game::Vitality", &[("hp", FieldKind::F32)])]);

    let report = Reloaded::between(&before, &after);

    assert_eq!(report.lost, vec!["game::Health".to_owned()]);
    assert_eq!(report.gained, vec!["game::Vitality".to_owned()]);
    assert!(report.changed.is_empty());
}

/// The common rebuild: a function body changed and the types did not.
/// This is what stops the notice firing on every reload.
#[test]
fn an_unchanged_library_is_quiet() {
    let before = registry(&[("game::Health", &[("hp", FieldKind::F32)])]);
    let after = registry(&[("game::Health", &[("hp", FieldKind::F32)])]);

    assert!(Reloaded::between(&before, &after).is_quiet());
}

#[test]
fn a_reordered_field_is_not_a_change() {
    let before = registry(&[("game::P", &[("a", FieldKind::F32), ("b", FieldKind::Bool)])]);
    let after = registry(&[("game::P", &[("b", FieldKind::Bool), ("a", FieldKind::F32)])]);

    assert!(Reloaded::between(&before, &after).is_quiet());
}

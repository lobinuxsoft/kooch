//! Opening a project starts a remote session; plugin types must survive it.

use ome_core::resource::Resources;
use ome_ecs::component::{
    ComponentNames, ComponentRegistry, DynamicField, DynamicType, DynamicTypeRegistry,
};
use ome_ecs::dynamic_components::DynamicComponents;
use ome_ecs::reflect::FieldKind;

use super::super::{gather_reflected_types, intern_registry_names};

fn world_with_plugin_type() -> Resources {
    let mut r = Resources::new();
    r.insert(ComponentRegistry::new());
    r.insert(DynamicComponents::new());
    r.insert(ComponentNames::new());

    let mut types = DynamicTypeRegistry::new();
    types
        .register(DynamicType {
            type_name: "move_component::MoveComponent".into(),
            fields: vec![DynamicField {
                name: "speed".into(),
                kind: FieldKind::F32,
            }],
            source: "move_component".into(),
        })
        .unwrap();
    r.insert(types);
    r
}

/// With no session at all the type is listed — the case that already
/// worked, kept so the union does not regress it.
#[test]
fn listed_without_a_session() {
    let mut resources = world_with_plugin_type();
    intern_registry_names(&mut resources);

    let names: Vec<String> = gather_reflected_types(&resources)
        .into_iter()
        .map(|t| t.short_name)
        .collect();
    assert!(names.contains(&"MoveComponent".to_owned()), "{names:?}");
}

/// And it must still be listed once a `RemoteState` exists, which is
/// what opening any project creates.
#[test]
fn still_listed_with_a_remote_state_present() {
    let mut resources = world_with_plugin_type();
    resources.insert(crate::remote_session::RemoteState::new());
    intern_registry_names(&mut resources);

    let names: Vec<String> = gather_reflected_types(&resources)
        .into_iter()
        .map(|t| t.short_name)
        .collect();
    assert!(
        names.contains(&"MoveComponent".to_owned()),
        "a plugin type vanished once a project was open: {names:?}"
    );
}

/// A type reported by both routes must appear once, not twice.
#[test]
fn not_listed_twice() {
    let mut resources = world_with_plugin_type();
    intern_registry_names(&mut resources);

    let count = gather_reflected_types(&resources)
        .iter()
        .filter(|t| t.short_name == "MoveComponent")
        .count();
    assert_eq!(count, 1, "duplicated in the menu");
}

//! The Components panel is the other route to adding a component.

use kooch_core::resource::Resources;
use kooch_ecs::component::{
    ComponentId, ComponentNames, ComponentRegistry, DynamicField, DynamicType, DynamicTypeRegistry,
};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::reflect::FieldKind;
use kooch_ecs::transform::Transform;

use super::super::{gather_component_types, intern_registry_names};

fn world() -> Resources {
    let mut r = Resources::new();
    r.insert(ComponentRegistry::new());
    r.insert(DynamicComponents::new());
    r.insert(ComponentNames::new());
    r.get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Transform>();

    let mut types = DynamicTypeRegistry::new();
    types
        .register(DynamicType {
            type_name: "move_component::MoveComponent".into(),
            fields: vec![DynamicField {
                name: "speed".into(),
                kind: FieldKind::F32,
            }],
            defaults: Vec::new(),
            source: "move_component".into(),
        })
        .unwrap();
    r.insert(types);
    r
}

#[test]
fn a_plugin_type_is_listed_for_drag_drop() {
    let mut resources = world();
    intern_registry_names(&mut resources);

    let rows = gather_component_types(&resources);
    let row = rows
        .iter()
        .find(|t| t.short_name == "MoveComponent")
        .expect("plugin type missing from the Components panel");

    assert_ne!(
        row.component,
        ComponentId::INVALID,
        "listed but not draggable — the name was never interned"
    );
    assert!(
        row.has_reflection,
        "its schema is known, so the panel must not mark it unreflected"
    );
    assert!(rows.iter().any(|t| t.short_name == "Transform"));
}

#[test]
fn it_is_not_listed_twice() {
    let mut resources = world();
    intern_registry_names(&mut resources);

    let count = gather_component_types(&resources)
        .iter()
        .filter(|t| t.short_name == "MoveComponent")
        .count();
    assert_eq!(count, 1);
}

//! Types a loaded plugin declared must reach the Add Component menu.

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{
    ComponentId, ComponentNames, ComponentRegistry, DynamicField, DynamicType, DynamicTypeRegistry,
};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::reflect::FieldKind;
use kooch_ecs::transform::Transform;

use super::super::{gather_reflected_types, intern_registry_names};

/// A local editor world with one plugin-declared type registered.
fn world_with_plugin_type() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    r.insert(DynamicComponents::new());
    r.insert(ComponentNames::new());
    r.get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Transform>();

    let mut types = DynamicTypeRegistry::new();
    types
        .register(DynamicType {
            type_name: "my_game::Health".into(),
            fields: vec![DynamicField {
                name: "current".into(),
                kind: FieldKind::U32,
            }],
            source: "my_game".into(),
        })
        .unwrap();
    r.insert(types);
    r
}

/// The whole point: a component the editor binary never compiled
/// shows up in the Add Component menu.
#[test]
fn a_plugin_type_appears_beside_the_engines_own() {
    let mut resources = world_with_plugin_type();
    intern_registry_names(&mut resources);

    let types = gather_reflected_types(&resources);
    let names: Vec<&str> = types.iter().map(|t| t.short_name.as_str()).collect();

    assert!(
        names.contains(&"Health"),
        "plugin type missing from {names:?}"
    );
    assert!(names.contains(&"Transform"), "engine types must remain");
}

/// Listing it is useless if it cannot be added: an un-interned name
/// resolves to INVALID and every action on it is dropped.
#[test]
fn its_component_id_resolves() {
    let mut resources = world_with_plugin_type();
    intern_registry_names(&mut resources);

    let types = gather_reflected_types(&resources);
    let health = types
        .iter()
        .find(|t| t.short_name == "Health")
        .expect("Health listed");

    assert_ne!(
        health.component,
        ComponentId::INVALID,
        "the name was never interned, so adding it would be dropped"
    );
}

/// Grouped by the plugin that brought them, so project components do
/// not scatter through the engine's list.
#[test]
fn it_is_categorised_by_its_source() {
    let mut resources = world_with_plugin_type();
    intern_registry_names(&mut resources);

    let types = gather_reflected_types(&resources);
    let health = types.iter().find(|t| t.short_name == "Health").unwrap();

    assert_eq!(health.category.as_deref(), Some("my_game"));
}

/// No plugins loaded must change nothing.
#[test]
fn without_the_registry_the_menu_is_unchanged() {
    let mut resources = world_with_plugin_type();
    resources.remove::<DynamicTypeRegistry>();
    intern_registry_names(&mut resources);

    let types = gather_reflected_types(&resources);
    assert!(types.iter().all(|t| t.short_name != "Health"));
    assert!(types.iter().any(|t| t.short_name == "Transform"));
}

//! List, add, inspect and edit a component the editor never compiled.

use std::collections::HashSet;

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{
    ComponentNames, ComponentRegistry, DynamicField, DynamicType, DynamicTypeRegistry,
};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::reflect::{FieldKind, InspectorVisibility, ReflectValue};

use crate::actions::EditorAction;

use super::super::{gather_entity_data, gather_reflected_types, intern_registry_names};

fn world() -> (Resources, kooch_ecs::Entity) {
    let mut r = Resources::new();
    let mut alloc = EntityAllocator::new();
    let entity = alloc.spawn();
    r.insert(alloc);
    r.insert(ComponentRegistry::new());
    let mut archetypes = ArchetypeRegistry::new();
    let empty = archetypes.get_or_create(Default::default());
    archetypes.register_entity(entity, empty);
    r.insert(archetypes);
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    r.insert(DynamicComponents::new());
    r.insert(ComponentNames::new());
    r.insert(crate::undo::UndoStack::new());

    let mut types = DynamicTypeRegistry::new();
    types
        .register(DynamicType {
            type_name: "my_game::Health".into(),
            fields: vec![DynamicField {
                name: "current".into(),
                kind: FieldKind::U32,
            }],
            defaults: Vec::new(),
            source: "my_game".into(),
        })
        .unwrap();
    r.insert(types);
    (r, entity)
}

#[test]
fn list_add_inspect_edit_undo() {
    let (mut resources, entity) = world();
    intern_registry_names(&mut resources);

    // 1. It is offered by the menu, with a usable id.
    let listed = gather_reflected_types(&resources);
    let health = listed
        .iter()
        .find(|t| t.short_name == "Health")
        .expect("Health must be listed");

    // 2. Adding it goes through the ordinary action path.
    let mut undo = resources.remove::<crate::undo::UndoStack>().unwrap();
    crate::actions::apply_actions(
        &mut resources,
        &[EditorAction::AddComponent {
            entity,
            component: health.component,
        }],
        &mut undo,
    );
    resources.insert(undo);

    // 3. The Inspector sees it, with its field, editable.
    let shown = gather_entity_data(&resources, &HashSet::from([entity]));
    let row = shown
        .iter()
        .find(|e| e.entity == entity)
        .expect("entity present");
    let comp = row
        .components
        .iter()
        .find(|c| c.short_name == "Health")
        .expect("Health must be on the entity");
    assert_eq!(
        comp.visibility,
        InspectorVisibility::Editable,
        "a project's own component must be editable in the editor that authors it"
    );
    let fields = comp.fields.values().expect("fields shown");
    assert_eq!(fields[0].0, "current");
    assert_eq!(fields[0].1, ReflectValue::U32(0));

    // 4. Editing it lands.
    let mut undo = resources.remove::<crate::undo::UndoStack>().unwrap();
    crate::actions::apply_actions(
        &mut resources,
        &[EditorAction::SetField {
            entity,
            component: health.component,
            field: "current".into(),
            value: ReflectValue::U32(80),
        }],
        &mut undo,
    );
    resources.insert(undo);
    let stored = resources
        .get::<DynamicComponents>()
        .unwrap()
        .iter_entity(entity)
        .find(|(n, _)| *n == "my_game::Health")
        .map(|(_, f)| f[0].1.clone())
        .unwrap();
    assert_eq!(stored, ReflectValue::U32(80), "the edit did not land");
}

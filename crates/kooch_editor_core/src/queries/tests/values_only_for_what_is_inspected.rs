//! Reflected values are read for the selection and skipped elsewhere.
//!
//! Reading them was 5.26 ms of a 610-entity frame — 97% of the gather
//! stage (#691) — for values only the Inspector reads, of the one entity
//! it shows. What matters here is that skipping them stays *legible*:
//! a component whose values were not read must not be indistinguishable
//! from one whose type has no reflection.

use std::collections::HashSet;

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::reflect::ReflectValue;

use crate::state::ReflectedFields;

use super::super::{gather_entity_data, intern_registry_names};

/// Two entities, each with a `Name` and a `Transform`.
fn world() -> (Resources, kooch_ecs::Entity, kooch_ecs::Entity) {
    use kooch_ecs::name::Name;
    use kooch_ecs::transform::Transform;

    let mut r = Resources::new();
    let mut alloc = EntityAllocator::new();
    let first = alloc.spawn();
    let second = alloc.spawn();
    r.insert(alloc);

    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Name>();
    registry.register_cpu_reflected::<Transform>();
    for (entity, name) in [(first, "First"), (second, "Second")] {
        registry
            .get_cpu_mut::<Name>()
            .expect("Name registered")
            .insert(entity, Name::new(name));
        registry
            .get_cpu_mut::<Transform>()
            .expect("Transform registered")
            .insert(entity, Transform::default());
    }

    let mut archetypes = ArchetypeRegistry::new();
    let signature = [
        std::any::TypeId::of::<Name>(),
        std::any::TypeId::of::<Transform>(),
    ]
    .into_iter()
    .collect();
    let id = archetypes.get_or_create(signature);
    archetypes.register_entity(first, id);
    archetypes.register_entity(second, id);

    r.insert(registry);
    r.insert(archetypes);
    r.insert(AccessTracker::new());
    r.insert(DynamicComponents::new());
    r.insert(ComponentNames::new());
    intern_registry_names(&mut r);
    (r, first, second)
}

fn component<'a>(
    entities: &'a [crate::state::EntityDisplayInfo],
    entity: kooch_ecs::Entity,
    name: &str,
) -> &'a crate::state::ComponentDisplayInfo {
    entities
        .iter()
        .find(|e| e.entity == entity)
        .expect("entity gathered")
        .components
        .iter()
        .find(|c| c.short_name == name)
        .expect("component gathered")
}

#[test]
fn the_selection_gets_its_values() {
    let (resources, first, _) = world();
    let entities = gather_entity_data(&resources, &HashSet::from([first]));
    assert!(
        component(&entities, first, "Transform")
            .fields
            .values()
            .is_some(),
        "the Inspector shows this entity and has nothing to show",
    );
}

#[test]
fn an_unselected_entity_does_not_pay_for_values_nobody_reads() {
    let (resources, first, second) = world();
    let entities = gather_entity_data(&resources, &HashSet::from([first]));
    assert!(matches!(
        component(&entities, second, "Transform").fields,
        ReflectedFields::NotGathered,
    ));
}

/// The whole reason for a third state. `NotGathered` and `Unreflected`
/// both have no values; only one of them means the component cannot be
/// edited, and a panel has to be able to tell them apart.
#[test]
fn not_gathered_is_not_the_same_as_unreflectable() {
    let (resources, first, second) = world();
    let entities = gather_entity_data(&resources, &HashSet::from([first]));
    let skipped = &component(&entities, second, "Transform").fields;

    assert!(skipped.values().is_none());
    assert!(
        skipped.is_reflectable(),
        "a skipped read must not read as a component without a schema",
    );
}

/// Every row in the hierarchy shows a name, so that one component is
/// read for everybody. Without this the World panel would list six
/// hundred rows of "Entity 412:0".
#[test]
fn every_entity_keeps_its_name_selected_or_not() {
    let (resources, first, second) = world();
    let entities = gather_entity_data(&resources, &HashSet::from([first]));

    for entity in [first, second] {
        let fields = component(&entities, entity, "Name")
            .fields
            .values()
            .expect("Name is read for every entity");
        assert!(
            fields
                .iter()
                .any(|(_, value)| matches!(value, ReflectValue::String(_))),
            "the name's value is what the row displays",
        );
    }
}

/// Skipping the values must not skip the component. The hierarchy's
/// component count, the prefab marker and "does this entity have a
/// Collider" all read the list, never the values.
#[test]
fn an_unselected_entity_still_lists_its_components() {
    let (resources, first, second) = world();
    let entities = gather_entity_data(&resources, &HashSet::from([first]));
    let listed: Vec<&str> = entities
        .iter()
        .find(|e| e.entity == second)
        .expect("entity gathered")
        .components
        .iter()
        .map(|c| c.short_name.as_str())
        .collect();
    assert!(listed.contains(&"Name"));
    assert!(listed.contains(&"Transform"));
}

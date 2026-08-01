//! Components come out in display order whichever list they came from.
//!
//! The order used to be produced by sorting every entity's list, every
//! frame — 610 sorts to reproduce the order the previous 609 already had
//! (#666). Now an archetype's components are sorted once, when the
//! archetype is resolved, and the per-entity sort runs only when there
//! is a parked component to merge in.
//!
//! That is two paths where there was one, and the failure they invite is
//! silent: the panel still draws, just with `Transform` above `Name` on
//! the entities that happen to carry something parked.

use std::collections::HashSet;

use kooch_core::resource::Resources;
use kooch_ecs::Entity;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::name::Name;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::transform::Transform;

use super::super::{gather_entity_data, intern_registry_names};

/// Two entities of the same archetype: `Transform`, `Name` and
/// `PerspectiveCamera`, declared in the order the display must *not*
/// keep. The camera is there so a parked `Health` has something to sort
/// *before* — appended blindly it would land after it.
fn world() -> (Resources, Entity, Entity) {
    let mut r = Resources::new();
    let mut alloc = EntityAllocator::new();
    let plain = alloc.spawn();
    let with_parked = alloc.spawn();
    r.insert(alloc);

    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<Name>();
    registry.register_cpu_reflected::<PerspectiveCamera>();

    let mut archetypes = ArchetypeRegistry::new();
    let signature = [
        std::any::TypeId::of::<Transform>(),
        std::any::TypeId::of::<Name>(),
        std::any::TypeId::of::<PerspectiveCamera>(),
    ]
    .into_iter()
    .collect();
    let id = archetypes.get_or_create(signature);
    for (entity, label) in [(plain, "Plain"), (with_parked, "Parked")] {
        registry
            .get_cpu_mut::<Transform>()
            .expect("Transform registered")
            .insert(entity, Transform::default());
        registry
            .get_cpu_mut::<Name>()
            .expect("Name registered")
            .insert(entity, Name::new(label));
        registry
            .get_cpu_mut::<PerspectiveCamera>()
            .expect("PerspectiveCamera registered")
            .insert(entity, PerspectiveCamera::default());
        archetypes.register_entity(entity, id);
    }

    // A component the editor has no Rust type for, on one entity only —
    // so the two entities of one archetype take the two different paths.
    let mut dynamic = DynamicComponents::new();
    dynamic.insert(
        with_parked,
        "my_game::Health",
        vec![("hp".to_owned(), ReflectValue::F32(1.0))],
    );

    r.insert(registry);
    r.insert(archetypes);
    r.insert(AccessTracker::new());
    r.insert(dynamic);
    r.insert(ComponentNames::new());
    intern_registry_names(&mut r);
    (r, plain, with_parked)
}

fn names_on(entities: &[crate::state::EntityDisplayInfo], entity: Entity) -> Vec<String> {
    entities
        .iter()
        .find(|e| e.entity == entity)
        .expect("entity gathered")
        .components
        .iter()
        .map(|c| c.short_name.to_string())
        .collect()
}

#[test]
fn an_archetypes_own_components_are_already_in_order() {
    let (resources, plain, _) = world();
    let entities = gather_entity_data(&resources, &HashSet::new());
    assert_eq!(
        names_on(&entities, plain),
        ["Name", "Transform", "PerspectiveCamera"]
    );
}

/// Merged into the order rather than tacked onto the end of it: `Health`
/// sorts before `PerspectiveCamera`, so an append would be visible.
#[test]
fn a_parked_component_joins_the_order_rather_than_replacing_it() {
    let (resources, _, with_parked) = world();
    let entities = gather_entity_data(&resources, &HashSet::new());
    assert_eq!(
        names_on(&entities, with_parked),
        ["Name", "Transform", "Health", "PerspectiveCamera"]
    );
}

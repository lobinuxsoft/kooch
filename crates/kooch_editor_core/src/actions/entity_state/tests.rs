use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::entity::Entity;
use kooch_ecs::name::Name;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::transform::Transform;

use super::*;

fn world() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    r.insert(DynamicComponents::new());
    {
        let registry = r.get_mut::<ComponentRegistry>().unwrap();
        registry.register_cpu_reflected::<Name>();
        registry.register_cpu_reflected::<Transform>();
        registry.register_cpu_reflected::<kooch_ecs::hierarchy::Parent>();
    }
    r
}

fn spawn(resources: &mut Resources) -> Entity {
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    resources.insert(commands);
    entity
}

fn add<T: 'static>(resources: &mut Resources, entity: Entity) {
    let type_id = std::any::TypeId::of::<T>();
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    assert!(registry.insert_default_reflected(&type_id, entity));
}

fn set(
    resources: &mut Resources,
    entity: Entity,
    ty: std::any::TypeId,
    field: &str,
    v: ReflectValue,
) {
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.reflect_set_field(&ty, entity, field, v).unwrap();
}

/// The values come back, not just the component types.
#[test]
fn a_capture_carries_its_values() {
    let mut resources = world();
    let entity = spawn(&mut resources);
    add::<Transform>(&mut resources, entity);
    set(
        &mut resources,
        entity,
        std::any::TypeId::of::<Transform>(),
        "position",
        ReflectValue::Vec3(glam::Vec3::new(1.0, 2.0, 3.0)),
    );

    let state = capture(&resources, entity);
    let transform = state
        .components
        .iter()
        .find(|c| c.name.ends_with("Transform"))
        .expect("the transform was not captured");
    assert!(transform.fields.contains(&(
        "position".to_owned(),
        ReflectValue::Vec3(glam::Vec3::new(1.0, 2.0, 3.0)),
    )));
}

/// 🔴 The hierarchy link is never a captured component. It travels as
/// its own field on both sides of the wire, and a copy that carried its
/// source's `Parent` as a value would point at whatever entity handle
/// happened to be at that index in the other process.
#[test]
fn the_parent_link_stays_home() {
    let mut resources = world();
    let entity = spawn(&mut resources);
    add::<Transform>(&mut resources, entity);
    add::<kooch_ecs::hierarchy::Parent>(&mut resources, entity);

    let state = capture(&resources, entity);
    assert!(
        !state.components.iter().any(|c| c.name.ends_with("Parent")),
        "the parent link was captured: {:?}",
        state.components.iter().map(|c| &c.name).collect::<Vec<_>>(),
    );
}

/// A component only the project knows is parked, not lost — it is
/// exactly the half a user wrote themselves.
#[test]
fn a_parked_component_is_captured() {
    let mut resources = world();
    let entity = spawn(&mut resources);
    resources.get_mut::<DynamicComponents>().unwrap().insert(
        entity,
        "the_game::Health",
        vec![("hp".to_owned(), ReflectValue::F32(7.0))],
    );

    let state = capture(&resources, entity);
    let health = state
        .components
        .iter()
        .find(|c| c.name == "the_game::Health")
        .expect("the parked component was not captured");
    assert_eq!(health.fields[0].1, ReflectValue::F32(7.0));
}

/// Captured here, restored there: the round trip is what paste and
/// undoing a despawn both rest on.
#[test]
fn a_restored_entity_matches() {
    let mut resources = world();
    let source = spawn(&mut resources);
    add::<Transform>(&mut resources, source);
    set(
        &mut resources,
        source,
        std::any::TypeId::of::<Transform>(),
        "scale",
        ReflectValue::Vec3(glam::Vec3::splat(4.0)),
    );
    let state = capture(&resources, source);

    let copy = spawn(&mut resources);
    restore_local(&mut resources, copy, &state);

    let scale = resources
        .get::<ComponentRegistry>()
        .unwrap()
        .reflect_get_fields(&std::any::TypeId::of::<Transform>(), copy)
        .expect("the copy has no transform")
        .into_iter()
        .find(|(name, _)| name == "scale")
        .map(|(_, value)| value);
    assert_eq!(scale, Some(ReflectValue::Vec3(glam::Vec3::splat(4.0))));
}

/// A copy says it is one, and an unnamed entity produces no name at all
/// rather than the string "Copy".
#[test]
fn a_copy_is_named_after_it() {
    let named = EntityState {
        name: Some("Hero".to_owned()),
        components: Vec::new(),
    };
    assert_eq!(copy_name(&named).as_deref(), Some("Hero Copy"));
    assert_eq!(copy_name(&EntityState::default()), None);
}

/// 🔴 A copy carries what the entity IS, not which file it came out of.
///
/// `capture` takes every reflected component and `SceneMember` is one,
/// so the copy used to name its source scene — and restoring it wrote
/// that scene over wherever the paste had just placed the entity. The
/// symptom was a paste that ignored the scene it was asked for.
#[test]
fn a_copy_does_not_carry_its_scene() {
    let state = EntityState {
        name: Some("Hero".to_owned()),
        components: vec![
            ComponentState {
                name: std::any::type_name::<kooch_ecs::SceneMember>().to_owned(),
                fields: vec![(
                    "scene".to_owned(),
                    ReflectValue::String(kooch_core::Guid::new_v4().to_string()),
                )],
            },
            ComponentState {
                name: std::any::type_name::<Transform>().to_owned(),
                fields: Vec::new(),
            },
        ],
    };

    let copy = as_copy(&state);

    let names: Vec<&str> = copy.components.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec![std::any::type_name::<Transform>()]);
}

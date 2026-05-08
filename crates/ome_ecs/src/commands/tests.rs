//! Unit tests for the deferred command buffer.

use super::*;
use crate::allocator::EntityAllocator;
use crate::archetype_registry::ArchetypeRegistry;
use crate::component::registry::ComponentRegistry;
use crate::component::traits::{Component, GpuComponent};
use crate::query::{Query, With, Without};
use ome_core::resource::Resources;

// -- Test components --

struct Health(u32);
impl Component for Health {}

struct Name(String);
impl Component for Name {}

struct Marker;
impl Component for Marker {}

#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Position {
    x: f32,
    y: f32,
}
impl GpuComponent for Position {}

fn setup() -> Resources {
    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(crate::query::AccessTracker::new());
    resources
}

// -- Spawn tests --

#[test]
fn spawn_single_entity() {
    let mut resources = setup();
    let mut commands = Commands::new();

    let entity = commands
        .spawn(&mut resources)
        .insert(Health(100))
        .id();

    assert!(entity.is_valid());
    commands.apply(&mut resources);

    let query = Query::<&Health>::new(&resources);
    let health = query.get(entity).unwrap();
    assert_eq!(health.0, 100);
}

#[test]
fn spawn_multiple_components() {
    let mut resources = setup();
    let mut commands = Commands::new();

    let entity = commands
        .spawn(&mut resources)
        .insert(Health(100))
        .insert(Name("Player".into()))
        .id();

    commands.apply(&mut resources);

    let query = Query::<(&Health, &Name)>::new(&resources);
    let (health, name) = query.get(entity).unwrap();
    assert_eq!(health.0, 100);
    assert_eq!(name.0, "Player");
}

#[test]
fn spawn_with_gpu_component() {
    let mut resources = setup();
    let mut commands = Commands::new();

    let entity = commands
        .spawn(&mut resources)
        .insert(Health(100))
        .insert_gpu(Position { x: 1.0, y: 2.0 })
        .id();

    commands.apply(&mut resources);

    let query = Query::<(&Health, &Position)>::new(&resources);
    let (health, pos) = query.get(entity).unwrap();
    assert_eq!(health.0, 100);
    assert_eq!(pos.x, 1.0);
    assert_eq!(pos.y, 2.0);
}

#[test]
fn spawn_without_id() {
    let mut resources = setup();
    let mut commands = Commands::new();

    // Just drop the builder — entity still gets created.
    commands
        .spawn(&mut resources)
        .insert(Health(42));

    commands.apply(&mut resources);

    let query = Query::<&Health>::new(&resources);
    let health = query.iter().next().unwrap();
    assert_eq!(health.0, 42);
}

#[test]
fn spawn_batch_entities() {
    let mut resources = setup();
    let mut commands = Commands::new();

    let entities = commands.spawn_batch(&mut resources, 5, |i, builder| {
        builder.insert(Health(i as u32 * 10));
    });

    assert_eq!(entities.len(), 5);
    commands.apply(&mut resources);

    let query = Query::<&Health>::new(&resources);
    let total: u32 = query.iter().map(|h| h.0).sum();
    assert_eq!(total, 0 + 10 + 20 + 30 + 40);
}

// -- Despawn tests --

#[test]
fn despawn_entity() {
    let mut resources = setup();
    let mut commands = Commands::new();

    let entity = commands
        .spawn(&mut resources)
        .insert(Health(100))
        .id();
    commands.apply(&mut resources);

    // Verify entity exists.
    assert!(Query::<&Health>::new(&resources).get(entity).is_some());

    // Despawn.
    commands.despawn(entity);
    commands.apply(&mut resources);

    // Entity should be gone.
    let query = Query::<&Health>::new(&resources);
    assert!(query.get(entity).is_none());
    assert!(query.is_empty());
}

#[test]
fn despawn_via_entity_commands() {
    let mut resources = setup();
    let mut commands = Commands::new();

    let entity = commands
        .spawn(&mut resources)
        .insert(Health(100))
        .id();
    commands.apply(&mut resources);

    commands.entity(entity).despawn();
    commands.apply(&mut resources);

    assert!(Query::<&Health>::new(&resources).is_empty());
}

// -- Insert/Remove on existing entity --

#[test]
fn insert_component_on_existing_entity() {
    let mut resources = setup();
    let mut commands = Commands::new();

    let entity = commands.spawn(&mut resources).insert(Health(100)).id();
    commands.apply(&mut resources);

    // Add a new component later.
    commands.entity(entity).insert(Name("Updated".into()));
    commands.apply(&mut resources);

    let query = Query::<(&Health, &Name)>::new(&resources);
    let (health, name) = query.get(entity).unwrap();
    assert_eq!(health.0, 100);
    assert_eq!(name.0, "Updated");
}

#[test]
fn remove_component_from_entity() {
    let mut resources = setup();
    let mut commands = Commands::new();

    let entity = commands
        .spawn(&mut resources)
        .insert(Health(100))
        .insert(Marker)
        .id();
    commands.apply(&mut resources);

    // Remove Marker component.
    commands.entity(entity).remove::<Marker>();
    commands.apply(&mut resources);

    // Entity should still have Health but not Marker.
    let query = Query::<&Health, Without<Marker>>::new(&resources);
    assert!(query.get(entity).is_some());

    let query = Query::<&Health, With<Marker>>::new(&resources);
    assert!(query.get(entity).is_none());
}

// -- Apply system --

#[test]
fn commands_apply_system_works() {
    let mut resources = setup();
    resources.insert(Commands::new());

    // Spawn via the resource.
    {
        let mut commands = resources.remove::<Commands>().unwrap();
        commands.spawn(&mut resources).insert(Health(77));
        resources.insert(commands);
    }

    commands_apply_system(&mut resources);

    let query = Query::<&Health>::new(&resources);
    assert_eq!(query.iter().next().unwrap().0, 77);
}

#[test]
fn empty_commands_apply_is_noop() {
    let mut resources = setup();
    let mut commands = Commands::new();
    commands.apply(&mut resources); // Should not panic.
}

// -- Edge cases --

#[test]
fn spawn_and_despawn_same_batch() {
    let mut resources = setup();
    let mut commands = Commands::new();

    let entity = commands.spawn(&mut resources).insert(Health(100)).id();
    commands.despawn(entity);
    commands.apply(&mut resources);

    // Entity was spawned then immediately despawned.
    assert!(Query::<&Health>::new(&resources).is_empty());
}

#[test]
fn multiple_apply_calls() {
    let mut resources = setup();
    let mut commands = Commands::new();

    let e1 = commands.spawn(&mut resources).insert(Health(10)).id();
    commands.apply(&mut resources);

    let e2 = commands.spawn(&mut resources).insert(Health(20)).id();
    commands.apply(&mut resources);

    let query = Query::<&Health>::new(&resources);
    assert_eq!(query.get(e1).unwrap().0, 10);
    assert_eq!(query.get(e2).unwrap().0, 20);
}

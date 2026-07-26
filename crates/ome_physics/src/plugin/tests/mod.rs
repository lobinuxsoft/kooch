//! Acceptance tests for the ECS↔solver bridge (#139).
//!
//! Each test maps to one line of the issue's acceptance list, plus the
//! interaction the issue flags as unconsidered: what `WorldSnapshot`'s
//! restore does to a solver that does not know it exists.
//!
//! This module is the shared harness; the assertions live in the
//! submodules below.

mod compound;
mod configuration;
mod joints;
mod lifetime;
mod play_lifecycle;
mod simulation;

use std::any::TypeId;

use glam::{Quat, Vec3};

use ome_core::resource::Resources;
use ome_core::run_state::Playing;
use ome_core::time::Time;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::commands::Commands;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::dynamic_components::DynamicComponents;
use ome_ecs::entity::Entity;
use ome_ecs::query::AccessTracker;
use ome_ecs::transform::Transform;
use ome_ecs::world_snapshot::WorldSnapshot;

use crate::backend::CollisionShape;
use crate::components::{Collider, Joint, KIND_KINEMATIC, KIND_STATIC, RigidBody, SHAPE_CUBOID};
use crate::rapier_backend::RapierBackend;

use super::systems::{physics_step_system, physics_sync_system, physics_writeback_system};
use super::world::{BodySpec, PhysicsBody, PhysicsWorld};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A world with the ECS resources, a physics world, and time, not playing.
fn world() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    r.insert(DynamicComponents::new());
    r.insert(Time::new());
    r.insert(PhysicsWorld::new(Box::new(RapierBackend::new())));

    let registry = r.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<RigidBody>();
    registry.register_cpu_reflected::<Collider>();
    registry.register_cpu_reflected::<Joint>();
    registry.register_cpu::<PhysicsBody>();
    // The hierarchy the compound walk reads. Normally registered by
    // EcsPlugin; this harness builds its Resources by hand.
    registry.register_cpu_reflected::<ome_ecs::hierarchy::Parent>();
    registry.register_cpu_reflected::<ome_ecs::hierarchy::Children>();
    registry.register_cpu_reflected::<ome_ecs::hierarchy::GlobalTransform>();
    r
}

/// Spawns an entity carrying `Transform`, `RigidBody` and `Collider`.
fn spawn_body(
    resources: &mut Resources,
    transform: Transform,
    body: RigidBody,
    collider: Collider,
) -> Entity {
    let entity = spawn_bare(resources);
    insert(resources, entity, transform);
    insert(resources, entity, body);
    insert(resources, entity, collider);
    entity
}

/// Spawns an entity with no components.
fn spawn_bare(resources: &mut Resources) -> Entity {
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    resources.insert(commands);
    entity
}

/// Inserts a component and moves the entity's archetype along.
fn insert<T: ome_ecs::component::Component>(resources: &mut Resources, entity: Entity, value: T) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<T>()
    {
        storage.insert(entity, value);
    }
    let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() else {
        return;
    };
    let current = match archetypes.entity_archetype(entity) {
        Some(current) => current,
        None => {
            let empty = archetypes.get_or_create(Default::default());
            archetypes.register_entity(entity, empty);
            empty
        }
    };
    let next = archetypes.archetype_after_add_dynamic(current, TypeId::of::<T>());
    archetypes.register_entity(entity, next);
}

/// Removes a component, archetype included.
fn remove<T: 'static>(resources: &mut Resources, entity: Entity) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.remove_component(entity, &TypeId::of::<T>());
    }
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let next = archetypes.archetype_after_remove::<T>(current);
        archetypes.register_entity(entity, next);
    }
}

fn position(resources: &Resources, entity: Entity) -> Vec3 {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(entity))
        .map(|t| t.position)
        .expect("entity has no Transform")
}

fn body_count(resources: &Resources) -> usize {
    resources.get::<PhysicsWorld>().unwrap().len()
}

fn slot_of(resources: &Resources, entity: Entity) -> Option<u32> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<PhysicsBody>()?
        .get(entity)
        .map(PhysicsBody::slot)
}

/// The geometry the solver was actually built with, scale folded in.
fn shape_of(resources: &Resources, entity: Entity) -> CollisionShape {
    resources
        .get::<PhysicsWorld>()
        .unwrap()
        .spec(slot_of(resources, entity).expect("entity has no body"))
        .expect("slot is free")
        .desc(Vec3::ZERO, Quat::IDENTITY)
        .shape
}

/// The pose the solver actually holds for an entity's body.
fn solver_position(resources: &Resources, entity: Entity) -> Vec3 {
    let world = resources.get::<PhysicsWorld>().unwrap();
    let handle = world
        .handle(slot_of(resources, entity).expect("entity has no body"))
        .expect("slot is free");
    world
        .backend()
        .get_transform(handle)
        .expect("stale handle")
        .0
}

/// Runs `steps` frames: sync every frame, step + writeback while playing.
fn simulate(resources: &mut Resources, steps: u32) {
    for _ in 0..steps {
        physics_sync_system(resources);
        if Playing::is_playing(resources) {
            physics_step_system(resources);
            physics_writeback_system(resources);
        }
    }
}

/// A metre-scale dynamic unit sphere, dropped from `height`.
fn falling_sphere(resources: &mut Resources, height: f32) -> Entity {
    spawn_body(
        resources,
        Transform::from_position(Vec3::new(0.0, height, 0.0)),
        RigidBody::default(),
        Collider::default(),
    )
}

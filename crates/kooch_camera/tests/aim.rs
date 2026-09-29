//! A third-person aim against the real solver (#1370): the point is what the ray hits, and the
//! character's own body is never it.
//!
//! Run with:
//!   cargo test -p kooch_camera --features physics --test aim
#![cfg(feature = "physics")]

use glam::Vec3;

use kooch_camera::ThirdPersonAim;
use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::entity::Entity;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::transform::Transform;
use kooch_physics::components::{Collider, KIND_STATIC, PhysicsBody, SHAPE_CUBOID};
use kooch_physics::plugin::{PhysicsWorld, physics_step_system, physics_sync_system};
use kooch_physics::rapier_backend::RapierBackend;

fn world() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    r.insert(DynamicComponents::new());
    r.insert(kooch_core::time::Time::new());
    r.insert(PhysicsWorld::new(Box::new(RapierBackend::new())));
    kooch_core::run_state::Playing::set(&mut r, true);
    let registry = r.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<PhysicsBody>();
    registry.register_cpu_reflected::<Collider>();
    registry.register_cpu::<kooch_physics::plugin::SolverBody>();
    r
}

fn insert<T: Component>(resources: &mut Resources, entity: Entity, value: T) {
    use std::any::TypeId;
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

/// A 4 m cube at `at`, and the solver told about it.
fn wall(resources: &mut Resources, at: Vec3) {
    let mut commands = resources.remove::<Commands>().unwrap();
    let wall = commands.spawn(resources).id();
    commands.apply(resources);
    resources.insert(commands);
    insert(resources, wall, Transform::from_position(at));
    insert(
        resources,
        wall,
        PhysicsBody {
            kind: KIND_STATIC,
            ..Default::default()
        },
    );
    insert(
        resources,
        wall,
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(2.0),
            ..Default::default()
        },
    );
    physics_sync_system(resources);
    physics_step_system(resources);
}

fn aim() -> ThirdPersonAim {
    ThirdPersonAim {
        aim_distance: 100.0,
        ..Default::default()
    }
}

/// The point is where the ray lands, not where the reach would have put it.
#[test]
fn a_wall_is_what_it_aims_at() {
    let mut resources = world();
    wall(&mut resources, Vec3::new(0.0, 0.0, -20.0));
    let at = aim().resolved(&resources, Vec3::Z * 3.0, Vec3::ZERO, Vec3::NEG_Z);
    // The cube's near face is 2 m this side of its centre.
    assert!((at.z + 18.0).abs() < 0.1, "{at:?}");
}

/// 🔴 And with nothing in the way it is the reach, so a reticle on empty sky still means something.
#[test]
fn an_empty_sky_reaches_full() {
    let resources = world();
    let at = aim().resolved(&resources, Vec3::Z * 3.0, Vec3::ZERO, Vec3::NEG_Z);
    assert!((at.z + 97.0).abs() < 0.1, "{at:?}");
}

/// A wall between the camera and the character is not what the character is aiming at: the ray
/// starts level with them, so their own cover never answers it.
#[test]
fn cover_behind_the_character_is_ignored() {
    let mut resources = world();
    wall(&mut resources, Vec3::new(0.0, 0.0, 6.0));
    let at = aim().resolved(&resources, Vec3::Z * 10.0, Vec3::ZERO, Vec3::NEG_Z);
    assert!(
        at.z < 0.0,
        "it aimed at the wall behind the character: {at:?}"
    );
}

//! A spring arm and a wall, with the real solver (#1251): the camera stops short of what stands
//! between it and the target, and the target's own body never counts.
//!
//! Run with:
//!   cargo test -p kooch_camera --features physics --test occlusion
#![cfg(feature = "physics")]

use glam::Vec3;

use kooch_camera::{
    CameraCollision, CameraTarget, FOLLOW_THIRD_PERSON, LOOK_AT_SIMPLE, VirtualCamera,
    drive_virtual_cameras,
};
use kooch_core::resource::Resources;
use kooch_core::run_state::Playing;
use kooch_core::time::Time;
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

/// How far back the rig wants the camera.
const ARM: f32 = 8.0;

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
    Playing::set(&mut r, true);

    let registry = r.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<PhysicsBody>();
    registry.register_cpu_reflected::<Collider>();
    registry.register_cpu::<kooch_physics::plugin::SolverBody>();
    registry.register_cpu_reflected::<VirtualCamera>();
    registry.register_cpu_reflected::<CameraTarget>();
    registry.register_cpu_reflected::<CameraCollision>();
    r
}

fn spawn(resources: &mut Resources) -> Entity {
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    resources.insert(commands);
    entity
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

/// Takes a component off, archetype included — what the sync reads to retire a body.
fn remove<T: 'static>(resources: &mut Resources, entity: Entity) {
    use std::any::TypeId;
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
        .and_then(|registry| registry.get_cpu::<Transform>()?.get(entity).copied())
        .expect("a transform")
        .position
}

/// A frame: the solver learns what exists, then the rig plans.
fn frame(resources: &mut Resources) {
    physics_sync_system(resources);
    physics_step_system(resources);
    drive_virtual_cameras(resources);
}

/// A player-sized body the camera follows, with a body of its own that must never stop the camera,
/// and a vcam an arm's length behind it. Answers the target and the vcam.
fn rig(collision: bool) -> (Resources, Entity, Entity) {
    rig_damped(collision, false)
}

/// The same, with the rig's own damping on or off.
fn rig_damped(collision: bool, damping: bool) -> (Resources, Entity, Entity) {
    let mut resources = world();
    let target = spawn(&mut resources);
    insert(&mut resources, target, Transform::from_position(Vec3::ZERO));
    insert(&mut resources, target, CameraTarget::default());
    insert(
        &mut resources,
        target,
        PhysicsBody {
            kind: KIND_STATIC,
            ..Default::default()
        },
    );
    insert(&mut resources, target, Collider::default());

    let vcam = spawn(&mut resources);
    insert(&mut resources, vcam, Transform::from_position(Vec3::ZERO));
    insert(
        &mut resources,
        vcam,
        VirtualCamera {
            follow: FOLLOW_THIRD_PERSON,
            look_at: LOOK_AT_SIMPLE,
            distance: ARM,
            damping,
            damping_duration: Vec3::splat(0.5),
            ..Default::default()
        },
    );
    if collision {
        insert(&mut resources, vcam, CameraCollision::default());
    }
    frame(&mut resources);
    (resources, target, vcam)
}

/// A 4 m cube halfway along the arm.
fn wall(resources: &mut Resources, at: Vec3) -> Entity {
    let wall = spawn(resources);
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
    wall
}

/// 🔴 The point of the issue: a wall between the target and where the rig wants the camera stops
/// the camera in front of it — and the target's own body, which the sweep starts inside, does not.
#[test]
fn a_wall_pulls_the_camera_in() {
    let (mut resources, target, vcam) = rig(true);
    let free = position(&resources, vcam) - position(&resources, target);
    assert!(
        (free.length() - ARM).abs() < 0.1,
        "with nothing in the way the arm should be whole, not {}",
        free.length(),
    );

    // The wall's near face 2 m from the target, its far face 6 m out: the camera cannot be past 2.
    wall(&mut resources, free.normalize() * 4.0);
    frame(&mut resources);
    let held = position(&resources, vcam).distance(position(&resources, target));
    assert!(
        held < 2.0,
        "the camera sat {held} m out, inside or past the wall"
    );
    assert!(held >= 0.5 - 1e-3, "closer than min_distance: {held}");
}

/// Without the component the arm goes through the wall, as it always did: the feature is opt-in.
#[test]
fn no_collision_goes_through() {
    let (mut resources, target, vcam) = rig(false);
    let free = position(&resources, vcam) - position(&resources, target);
    wall(&mut resources, free.normalize() * 4.0);
    frame(&mut resources);
    let held = position(&resources, vcam).distance(position(&resources, target));
    assert!((held - ARM).abs() < 0.1, "{held}");
}

/// 🔴 The return lasts `return_duration` with the rig's own damping on. The damping used to continue
/// from where the wall had put the camera, so the rig itself crept back out at its own pace and the
/// return's seconds came on top — a 0.35 s return that took over a second.
#[test]
fn a_return_takes_its_seconds_with_damping() {
    let (mut resources, target, vcam) = rig_damped(true, true);
    // Settle the damped rig at its full arm first.
    for _ in 0..600 {
        frame(&mut resources);
    }
    let free = position(&resources, vcam) - position(&resources, target);
    assert!(
        (free.length() - ARM).abs() < 0.1,
        "the rig never settled: {}",
        free.length()
    );

    let blocker = wall(&mut resources, free.normalize() * 4.0);
    frame(&mut resources);
    let pulled = position(&resources, vcam).distance(position(&resources, target));
    assert!(pulled < 2.0, "{pulled}");

    // The wall leaves; the return has its 0.35 s and a frame of slack, at 60 frames a second.
    remove::<PhysicsBody>(&mut resources, blocker);
    remove::<Collider>(&mut resources, blocker);
    for _ in 0..23 {
        frame(&mut resources);
    }
    let back = position(&resources, vcam).distance(position(&resources, target));
    assert!(
        (back - ARM).abs() < 0.1,
        "0.38 s after the wall left the arm was {back}, not {ARM}"
    );
}

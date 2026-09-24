//! A region authors itself (#1222): a post-process volume with a collider and nothing else is in
//! the solver, overlapping rather than pushing, and heard.

use super::*;

use kooch_core::event::Events;
use kooch_ecs::post_process_volume::PostProcessVolume;
use kooch_ecs::sensor_occupancy::SensorOccupancy;

use crate::components::KIND_DYNAMIC;
use crate::plugin::{CollisionStarted, CollisionStopped, ContactForce, JointBroke};

/// The harness builds `Resources` by hand, so what the plugin would have inserted goes here.
fn volume_world() -> Resources {
    let mut resources = world();
    resources.insert(Events::<CollisionStarted>::default());
    resources.insert(Events::<CollisionStopped>::default());
    resources.insert(Events::<ContactForce>::default());
    resources.insert(Events::<JointBroke>::default());
    resources.insert(SensorOccupancy::default());
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<PostProcessVolume>();
    }
    resources
}

/// One frame, scheduled the way the plugin schedules it — including the two cadences: the fixed
/// buffers swap as the fixed stages begin, the frame ones at the end of the frame (#1312).
fn frame(resources: &mut Resources) {
    crate::plugin::events::physics_lifecycle_system(resources);
    physics_sync_system(resources);
    if Playing::is_playing(resources) {
        if let Some(events) = resources.get_mut::<Events<CollisionStarted>>() {
            events.update_fixed();
        }
        if let Some(events) = resources.get_mut::<Events<CollisionStopped>>() {
            events.update_fixed();
        }
        physics_step_system(resources);
        physics_writeback_system(resources);
        crate::plugin::events::drain_physics_events(resources);
        crate::plugin::sensors::sensor_occupancy_system(resources);
    }
    if let Some(events) = resources.get_mut::<Events<CollisionStarted>>() {
        events.update();
    }
    if let Some(events) = resources.get_mut::<Events<CollisionStopped>>() {
        events.update();
    }
}

/// A 10 m box volume at the origin, authored the way the Inspector authors it: a shape, and the
/// component. No body, no sensor tick, no events tick.
fn volume(resources: &mut Resources) -> Entity {
    let entity = spawn_bare(resources);
    insert(resources, entity, Transform::default());
    // Normally the hierarchy propagates this; the harness runs the physics systems alone.
    insert(
        resources,
        entity,
        kooch_ecs::hierarchy::GlobalTransform::default(),
    );
    insert(
        resources,
        entity,
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(5.0, 5.0, 5.0),
            ..Default::default()
        },
    );
    insert(
        resources,
        entity,
        PostProcessVolume {
            blend_distance: 2.0,
            ..Default::default()
        },
    );
    entity
}

/// 🔴 The acceptance of the gate: a volume nobody configured for physics still reports. Every box
/// it needs — a body, `sensor`, `collision_events` — fails silently when it is missing, and an
/// author has no way to tell which one they forgot.
#[test]
fn a_volume_reports_without_being_configured() {
    let mut resources = volume_world();
    let region = volume(&mut resources);
    let body = spawn_body(
        &mut resources,
        Transform::default(),
        PhysicsBody {
            kind: KIND_KINEMATIC,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(0.5),
            ..Default::default()
        },
    );
    insert(
        &mut resources,
        body,
        kooch_ecs::hierarchy::GlobalTransform::default(),
    );
    Playing::set(&mut resources, true);
    for _ in 0..4 {
        frame(&mut resources);
    }

    let inside = resources
        .get::<SensorOccupancy>()
        .expect("the plugin keeps one");
    let depth = inside.depth_in(region).expect("the body is inside");
    assert!(
        inside.iter().any(|occupant| occupant.body == body),
        "the body is not in the region",
    );
    // At the centre of a 5 m half-extent box: five metres from the nearest face.
    assert!((depth - 5.0).abs() < 0.1, "it measured {depth}");
}

/// 🔴 #1298: the gate above only reached volumes with NO body of their own. Give one a
/// `PhysicsBody` — which the Inspector does the moment an author touches physics on it — and its
/// collider was taken exactly as authored: a solid box the solver never reports. Every volume in a
/// built game was dead, while the editor, which measures without a solver, showed it working.
#[test]
fn a_volume_with_its_own_body_still_reports() {
    let mut resources = volume_world();
    let region = volume(&mut resources);
    // What the Inspector leaves behind: a static body, and the two boxes nobody ticked.
    insert(
        &mut resources,
        region,
        PhysicsBody {
            kind: crate::components::KIND_STATIC,
            ..Default::default()
        },
    );
    let body = spawn_body(
        &mut resources,
        Transform::default(),
        PhysicsBody {
            kind: KIND_KINEMATIC,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(0.5),
            ..Default::default()
        },
    );
    insert(
        &mut resources,
        body,
        kooch_ecs::hierarchy::GlobalTransform::default(),
    );
    Playing::set(&mut resources, true);
    for _ in 0..4 {
        frame(&mut resources);
    }

    assert!(
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<PhysicsBody>())
            .and_then(|s| s.get(region))
            .is_some(),
        "the harness did not give the volume a body, so this proves nothing",
    );

    // The solver's own answer first: this is what used to be a solid box, and the occupancy below
    // only follows from it.
    let slot = slot_of(&resources, region).expect("the volume is in the solver");
    assert!(
        resources
            .get::<PhysicsWorld>()
            .and_then(|world| world.spec(slot))
            .expect("the slot has a spec")
            .is_sensor(),
        "a volume with a body of its own was authored as a solid collider",
    );

    let inside = resources
        .get::<SensorOccupancy>()
        .expect("the plugin keeps one");
    assert!(
        inside.iter().any(|occupant| occupant.body == body),
        "a volume with a body of its own never reported what walked into it",
    );
    let depth = inside.depth_in(region).expect("the body is inside");
    assert!((depth - 5.0).abs() < 0.1, "it measured {depth}");
}

/// A volume must not push what walks into it: a region that shoved the character out of itself
/// would be a wall with a colour grade.
#[test]
fn a_volume_pushes_nothing() {
    let mut resources = volume_world();
    volume(&mut resources);
    let body = spawn_body(
        &mut resources,
        Transform::default(),
        PhysicsBody {
            kind: KIND_DYNAMIC,
            gravity_scale: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(0.5),
            ..Default::default()
        },
    );
    Playing::set(&mut resources, true);
    for _ in 0..8 {
        frame(&mut resources);
    }
    let moved = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<Transform>()?.get(body).copied())
        .expect("the body has a transform")
        .position;
    assert!(
        moved.length() < 0.01,
        "the volume moved the body to {moved}",
    );
}

/// 🔴 #1302: a volume that listens to one layer heard the floor, because every collider was born on
/// **all** layers and "all" includes that one. A collider now names its layer and the project's
/// matrix decides the pairs.
#[test]
fn the_matrix_decides_who_meets_whom() {
    use kooch_core::layers::LayerNames;

    let mut resources = volume_world();
    // Player on layer 1, scenery on layer 0, and the two do not meet.
    let mut layers = LayerNames::default();
    layers.set(1, "Player");
    layers.set_collide(0, 1, false);
    resources.insert(layers);

    let region = volume(&mut resources);
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(colliders) = registry.get_cpu_mut::<Collider>()
        && let Some(collider) = colliders.get_mut(region)
    {
        collider.layer = 1;
    }
    let scenery = spawn_body(
        &mut resources,
        Transform::default(),
        PhysicsBody {
            kind: KIND_KINEMATIC,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(0.5),
            layer: 0,
            ..Default::default()
        },
    );
    insert(
        &mut resources,
        scenery,
        kooch_ecs::hierarchy::GlobalTransform::default(),
    );
    Playing::set(&mut resources, true);
    for _ in 0..4 {
        frame(&mut resources);
    }

    assert!(
        resources
            .get::<SensorOccupancy>()
            .expect("the plugin keeps one")
            .is_empty(),
        "the volume heard a layer the matrix says it never meets",
    );
}

/// The same volume hears what its layer does meet.
#[test]
fn the_matrix_lets_its_own_layer_through() {
    use kooch_core::layers::LayerNames;

    let mut resources = volume_world();
    let mut layers = LayerNames::default();
    layers.set(1, "Player");
    layers.set_collide(0, 1, false);
    resources.insert(layers);

    let region = volume(&mut resources);
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(colliders) = registry.get_cpu_mut::<Collider>()
        && let Some(collider) = colliders.get_mut(region)
    {
        collider.layer = 1;
    }
    let player = spawn_body(
        &mut resources,
        Transform::default(),
        PhysicsBody {
            kind: KIND_KINEMATIC,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(0.5),
            layer: 1,
            ..Default::default()
        },
    );
    insert(
        &mut resources,
        player,
        kooch_ecs::hierarchy::GlobalTransform::default(),
    );
    Playing::set(&mut resources, true);
    for _ in 0..4 {
        frame(&mut resources);
    }

    let inside = resources
        .get::<SensorOccupancy>()
        .expect("the plugin keeps one");
    assert!(
        inside.iter().any(|occupant| occupant.body == player),
        "the volume did not hear its own layer",
    );
}

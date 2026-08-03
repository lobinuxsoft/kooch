//! The interaction #139 flags as unconsidered: `WorldSnapshot` restores
//! the ECS on Stop and knows nothing about Rapier.

use super::*;

/// Acceptance: "Stop restores it exactly where it was, and it does not
/// keep drifting afterwards."
///
/// `WorldSnapshot` knows nothing about Rapier. What makes this work is
/// that `SolverBody` is unreflected: the restore wipes it, so the next
/// sync finds an entity with a `PhysicsBody` and no body, retires the stale
/// slot and builds a fresh one from the restored `Transform`. The ECS
/// stays the single source of truth — option A in the issue.
#[test]
fn stop_rebuilds_the_physics_world_from_the_restored_ecs() {
    let mut resources = world();
    let entity = falling_sphere(&mut resources, 10.0);
    physics_sync_system(&mut resources);
    let snapshot = WorldSnapshot::capture(&resources);

    Playing::set(&mut resources, true);
    simulate(&mut resources, 60);
    let fell_to = position(&resources, entity).y;
    assert!(fell_to < 9.0, "the body never fell, nothing to restore");

    // Stop.
    Playing::set(&mut resources, false);
    snapshot.restore(&mut resources);
    physics_sync_system(&mut resources);

    assert_eq!(
        position(&resources, entity),
        Vec3::new(0.0, 10.0, 0.0),
        "stop did not put the entity back"
    );
    assert_eq!(body_count(&resources), 1, "the play session leaked a body");

    // The rebuilt body starts at rest: without the rebuild it would keep
    // the velocity it built up over a second of falling.
    let slot = slot_of(&resources, entity).unwrap();
    let world_ref = resources.get::<PhysicsWorld>().unwrap();
    let handle = world_ref.handle(slot).unwrap();
    assert_eq!(
        world_ref.backend().linear_velocity(handle),
        Some(Vec3::ZERO),
        "the solver kept the velocity from the previous play session"
    );

    // And a second play repeats the first, from the top.
    Playing::set(&mut resources, true);
    simulate(&mut resources, 60);
    let fell_again = position(&resources, entity).y;
    assert!(
        (fell_again - fell_to).abs() < 0.5,
        "the second run diverged from the first: {fell_to} vs {fell_again}"
    );
}

/// An entity spawned during play is gone after stop, and so is its body.
#[test]
fn stop_releases_bodies_spawned_during_play() {
    let mut resources = world();
    falling_sphere(&mut resources, 10.0);
    physics_sync_system(&mut resources);
    let snapshot = WorldSnapshot::capture(&resources);

    Playing::set(&mut resources, true);
    falling_sphere(&mut resources, 20.0);
    simulate(&mut resources, 5);
    assert_eq!(body_count(&resources), 2);

    Playing::set(&mut resources, false);
    snapshot.restore(&mut resources);
    physics_sync_system(&mut resources);

    assert_eq!(
        body_count(&resources),
        1,
        "the runtime spawn's body survived the stop"
    );
}

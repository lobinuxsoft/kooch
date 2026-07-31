//! Per-body gravity: #624's cheap half.

use super::*;

/// Where a body ends up after two seconds, from a given scale.
fn falls_to(gravity_scale: f32) -> f32 {
    let mut resources = world();
    let entity = spawn_body(
        &mut resources,
        Transform::from_position(Vec3::new(0.0, 100.0, 0.0)),
        RigidBody {
            mass: 1.0,
            gravity_scale,
            ..Default::default()
        },
        Collider::default(),
    );
    Playing::set(&mut resources, true);
    simulate(&mut resources, 120);
    position(&resources, entity).y
}

/// Acceptance: "a body with `gravity_scale = 0` floats where it was left."
#[test]
fn zero_gravity_floats() {
    assert!(
        (falls_to(0.0) - 100.0).abs() < 1e-3,
        "a weightless body moved to {}",
        falls_to(0.0),
    );
}

/// Acceptance: "a body at 2 falls twice as fast as one at 1" — measured as
/// distance fallen, which under constant acceleration from rest is
/// proportional to it.
#[test]
fn double_gravity_falls_twice_as_far() {
    let normal = 100.0 - falls_to(1.0);
    let heavy = 100.0 - falls_to(2.0);

    assert!(normal > 1.0, "the normal body barely fell: {normal} m");
    assert!(
        (heavy / normal - 2.0).abs() < 0.05,
        "scale 2 fell {heavy} m against {normal} m — ratio {}",
        heavy / normal,
    );
}

/// A balloon. Negative is legal and is the whole reason this is a
/// multiplier rather than a switch.
#[test]
fn negative_gravity_rises() {
    assert!(
        falls_to(-1.0) > 100.0,
        "a negative scale did not rise: {}",
        falls_to(-1.0),
    );
}

/// Gravity is an acceleration, so mass must not change the outcome. A
/// heavier body falling slower would mean the scale had been folded in as
/// a force somewhere.
#[test]
fn mass_does_not_change_how_fast_a_body_falls() {
    fn fall_with_mass(mass: f32) -> f32 {
        let mut resources = world();
        let entity = spawn_body(
            &mut resources,
            Transform::from_position(Vec3::new(0.0, 100.0, 0.0)),
            RigidBody {
                mass,
                gravity_scale: 1.0,
                ..Default::default()
            },
            Collider::default(),
        );
        Playing::set(&mut resources, true);
        simulate(&mut resources, 120);
        position(&resources, entity).y
    }

    let light = fall_with_mass(1.0);
    let heavy = fall_with_mass(50.0);
    assert!(
        (light - heavy).abs() < 1e-2,
        "1 kg fell to {light} and 50 kg to {heavy}",
    );
}

/// Rapier bakes the scale into the body, so an Inspector edit has to
/// rebuild — the same seam every other body property crosses.
#[test]
fn editing_the_scale_reaches_the_solver() {
    let mut resources = world();
    let entity = spawn_body(
        &mut resources,
        Transform::default(),
        RigidBody::default(),
        Collider::default(),
    );
    physics_sync_system(&mut resources);
    let before = resources
        .get::<PhysicsWorld>()
        .unwrap()
        .spec(slot_of(&resources, entity).expect("no body"));

    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<RigidBody>()
        && let Some(body) = storage.get_mut(entity)
    {
        body.gravity_scale = 0.0;
    }
    physics_sync_system(&mut resources);

    let after = resources
        .get::<PhysicsWorld>()
        .unwrap()
        .spec(slot_of(&resources, entity).expect("no body"));
    assert_ne!(before, after, "the edit never reached the spec");
}

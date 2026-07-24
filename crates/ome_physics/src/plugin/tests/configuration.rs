//! Integration parameters, and entities authored only halfway.

use super::*;

#[test]
fn integration_parameters_round_trip() {
    let mut backend = RapierBackend::new();
    backend.set_length_unit(1000.0);
    backend.set_solver_iterations(8);

    assert_eq!(backend.length_unit(), 1000.0);
    assert_eq!(backend.solver_iterations(), 8);
}

/// Zero iterations would leave every contact unresolved, and a zero length
/// unit divides the solver's tolerances by nothing.
#[test]
fn degenerate_integration_parameters_are_clamped() {
    let mut backend = RapierBackend::new();
    backend.set_solver_iterations(0);
    backend.set_length_unit(0.0);

    assert!(backend.solver_iterations() >= 1);
    assert!(backend.length_unit() > 0.0);
}

/// An entity with a body and no collider still simulates, on the default
/// unit sphere — a half-authored entity should fall, not panic.
#[test]
fn a_body_without_a_collider_falls_on_the_default_shape() {
    let mut resources = world();
    let entity = spawn_bare(&mut resources);
    insert(
        &mut resources,
        entity,
        Transform::from_position(Vec3::new(0.0, 10.0, 0.0)),
    );
    insert(&mut resources, entity, RigidBody::default());

    Playing::set(&mut resources, true);
    simulate(&mut resources, 30);

    assert_eq!(body_count(&resources), 1);
    assert_eq!(
        resources
            .get::<PhysicsWorld>()
            .unwrap()
            .spec(slot_of(&resources, entity).unwrap())
            .unwrap()
            .desc(Vec3::ZERO, Quat::IDENTITY)
            .shape,
        CollisionShape::Sphere { radius: 0.5 }
    );
    assert!(position(&resources, entity).y < 9.9);
}

/// A body on an entity with no `Transform` starts at the origin rather
/// than refusing to exist.
#[test]
fn a_body_without_a_transform_starts_at_the_origin() {
    let mut resources = world();
    let entity = spawn_bare(&mut resources);
    insert(&mut resources, entity, RigidBody::default());

    physics_sync_system(&mut resources);

    assert_eq!(body_count(&resources), 1);
    assert_eq!(solver_position(&resources, entity), Vec3::ZERO);
}

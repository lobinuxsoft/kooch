//! Body lifetime: creation, release, slot reuse, and rebuilds driven by
//! an Inspector edit.

use super::*;

#[test]
fn a_rigid_body_gets_a_backend_body_and_a_slot() {
    let mut resources = world();
    let entity = falling_sphere(&mut resources, 10.0);

    physics_sync_system(&mut resources);

    assert_eq!(body_count(&resources), 1);
    let slot = slot_of(&resources, entity).expect("entity gained no PhysicsBody");
    assert_eq!(
        resources.get::<PhysicsWorld>().unwrap().entity(slot),
        Some(entity),
        "the reverse mapping does not point back at the entity"
    );
}

/// Sync is idempotent: repeated passes over an unchanged world must not
/// build a second body for the same entity.
#[test]
fn syncing_repeatedly_does_not_duplicate_bodies() {
    let mut resources = world();
    let first = falling_sphere(&mut resources, 10.0);
    let second = falling_sphere(&mut resources, 5.0);

    physics_sync_system(&mut resources);
    let slots = (slot_of(&resources, first), slot_of(&resources, second));
    physics_sync_system(&mut resources);
    physics_sync_system(&mut resources);

    assert_eq!(body_count(&resources), 2);
    assert_eq!(
        (slot_of(&resources, first), slot_of(&resources, second)),
        slots,
        "an unchanged world churned its slots"
    );
}

/// Acceptance: "Despawning an entity removes its Rapier body — body count
/// returns to zero."
#[test]
fn losing_the_rigid_body_releases_the_backend_body() {
    let mut resources = world();
    let entity = falling_sphere(&mut resources, 10.0);
    physics_sync_system(&mut resources);
    assert_eq!(body_count(&resources), 1);

    remove::<RigidBody>(&mut resources, entity);
    physics_sync_system(&mut resources);

    assert_eq!(body_count(&resources), 0, "the body leaked");
    assert_eq!(
        slot_of(&resources, entity),
        None,
        "the runtime component outlived its body"
    );
}

/// A freed slot is reused rather than growing the arrays forever.
#[test]
fn freed_slots_are_reused() {
    let mut resources = world();
    let first = falling_sphere(&mut resources, 10.0);
    physics_sync_system(&mut resources);
    let slot = slot_of(&resources, first).unwrap();

    remove::<RigidBody>(&mut resources, first);
    physics_sync_system(&mut resources);

    let second = falling_sphere(&mut resources, 10.0);
    physics_sync_system(&mut resources);

    assert_eq!(slot_of(&resources, second), Some(slot));
    assert_eq!(resources.get::<PhysicsWorld>().unwrap().capacity(), 1);
}

/// Editing the collider in the Inspector has to reach the solver. The
/// shape is baked into the Rapier body, so the body gets rebuilt.
#[test]
fn changing_the_authored_shape_rebuilds_the_body() {
    let mut resources = world();
    let entity = falling_sphere(&mut resources, 10.0);
    physics_sync_system(&mut resources);
    let before = resources
        .get::<PhysicsWorld>()
        .unwrap()
        .handle(slot_of(&resources, entity).unwrap());

    let edited = Collider {
        shape: SHAPE_CUBOID,
        half_extents: Vec3::splat(2.0),
        ..Default::default()
    };
    insert(&mut resources, entity, edited);
    physics_sync_system(&mut resources);

    let slot = slot_of(&resources, entity).unwrap();
    let after = resources.get::<PhysicsWorld>().unwrap().handle(slot);
    assert_eq!(body_count(&resources), 1, "the rebuild leaked a body");
    assert_ne!(before, after, "the solver kept the old shape");
    assert_eq!(
        resources.get::<PhysicsWorld>().unwrap().spec(slot).unwrap(),
        BodySpec::new(&RigidBody::default(), &edited)
    );
}

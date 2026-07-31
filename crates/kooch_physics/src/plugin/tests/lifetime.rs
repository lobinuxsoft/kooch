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
        BodySpec::new(&RigidBody::default(), &edited, Vec3::ONE)
    );
}

/// The bug this file's `scale` field exists for: a collider that ignored
/// `Transform.scale` worked at the authored size and nowhere else. Scale a
/// cube up with the gizmo and the mesh grew while the collider stayed
/// small — physics that looks broken "depending on the size".
///
/// A box scales exactly, per axis.
#[test]
fn a_scaled_cuboid_collider_grows_with_its_transform() {
    let mut resources = world();
    let entity = spawn_body(
        &mut resources,
        Transform {
            scale: Vec3::new(3.0, 1.0, 5.0),
            ..Transform::from_position(Vec3::ZERO)
        },
        RigidBody::default(),
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(0.5),
            ..Default::default()
        },
    );
    physics_sync_system(&mut resources);

    let shape = shape_of(&resources, entity);
    let CollisionShape::Cuboid { half_extents } = shape else {
        panic!("expected a cuboid, got {shape:?}");
    };
    assert!(
        half_extents.abs_diff_eq(Vec3::new(1.5, 0.5, 2.5), 1e-4),
        "the collider ignored the transform scale: {half_extents:?}"
    );
}

/// A sphere has no non-uniform form, so it takes the largest axis and
/// encloses the mesh. A collider smaller than what you can see is the one
/// that reads as a physics bug.
#[test]
fn a_scaled_sphere_collider_takes_the_largest_axis() {
    let mut resources = world();
    let entity = spawn_body(
        &mut resources,
        Transform {
            scale: Vec3::new(1.0, 4.0, 2.0),
            ..Transform::from_position(Vec3::ZERO)
        },
        RigidBody::default(),
        Collider::default(),
    );
    physics_sync_system(&mut resources);

    let shape = shape_of(&resources, entity);
    let CollisionShape::Sphere { radius } = shape else {
        panic!("expected a sphere, got {shape:?}");
    };
    assert!(
        (radius - 2.0).abs() < 1e-4,
        "expected 0.5 * 4.0 = 2.0, got {radius}"
    );
}

/// A capsule's radius follows its horizontal axes, not the largest: a tall
/// thin capsule scaled on Y should get taller, not fatter.
#[test]
fn a_scaled_capsule_grows_along_its_own_axis() {
    let mut resources = world();
    let entity = spawn_body(
        &mut resources,
        Transform {
            scale: Vec3::new(1.0, 3.0, 1.0),
            ..Transform::from_position(Vec3::ZERO)
        },
        RigidBody::default(),
        Collider {
            shape: crate::components::SHAPE_CAPSULE,
            radius: 0.5,
            half_height: 1.0,
            ..Default::default()
        },
    );
    physics_sync_system(&mut resources);

    let shape = shape_of(&resources, entity);
    let CollisionShape::Capsule {
        radius,
        half_height,
    } = shape
    else {
        panic!("expected a capsule, got {shape:?}");
    };
    assert!(
        (radius - 0.5).abs() < 1e-4,
        "scaling on Y made the capsule fatter: radius {radius}"
    );
    assert!(
        (half_height - 3.0).abs() < 1e-4,
        "expected 1.0 * 3.0 = 3.0, got {half_height}"
    );
}

/// Scaling an existing body rebuilds it. Rapier bakes dimensions into the
/// shape, so a scale change that does not rebuild changes nothing at all.
#[test]
fn scaling_an_existing_body_rebuilds_it() {
    let mut resources = world();
    let entity = falling_sphere(&mut resources, 10.0);
    physics_sync_system(&mut resources);
    let before = resources
        .get::<PhysicsWorld>()
        .unwrap()
        .handle(slot_of(&resources, entity).unwrap());

    insert(
        &mut resources,
        entity,
        Transform {
            position: Vec3::new(0.0, 10.0, 0.0),
            scale: Vec3::splat(4.0),
            ..Default::default()
        },
    );
    physics_sync_system(&mut resources);

    let after = resources
        .get::<PhysicsWorld>()
        .unwrap()
        .handle(slot_of(&resources, entity).unwrap());
    assert_ne!(before, after, "the scale change did not rebuild the body");
    assert_eq!(body_count(&resources), 1, "the rebuild leaked a body");
    let CollisionShape::Sphere { radius } = shape_of(&resources, entity) else {
        panic!("expected a sphere");
    };
    assert!((radius - 2.0).abs() < 1e-4, "got {radius}");
}

/// A scaled body still simulates: the point of all this is that a big
/// cube lands on a big floor at the right height.
#[test]
fn a_scaled_body_lands_on_a_scaled_floor() {
    let mut resources = world();
    // A 20x1x20 floor from a unit cuboid, scaled.
    spawn_body(
        &mut resources,
        Transform {
            scale: Vec3::new(20.0, 1.0, 20.0),
            ..Transform::from_position(Vec3::ZERO)
        },
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(0.5),
            ..Default::default()
        },
    );
    // A sphere of radius 0.5 scaled 2x = radius 1.0.
    let ball = spawn_body(
        &mut resources,
        Transform {
            scale: Vec3::splat(2.0),
            ..Transform::from_position(Vec3::new(0.0, 6.0, 0.0))
        },
        RigidBody::default(),
        Collider::default(),
    );
    Playing::set(&mut resources, true);
    simulate(&mut resources, 240);

    // Floor top at y = 0.5, ball radius 1.0 → rest at y ≈ 1.5.
    let resting = position(&resources, ball).y;
    assert!(
        (1.0..2.0).contains(&resting),
        "the scaled ball did not rest on the scaled floor: y = {resting}"
    );
}

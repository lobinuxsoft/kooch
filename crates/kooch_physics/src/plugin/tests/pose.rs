//! Where a parented body sits in the solver (#1316).

use super::*;

use kooch_ecs::hierarchy::Parent;

/// 🔴 The bug that killed the post-process volumes: a body was authored at its **local** transform,
/// so a volume parented under something 19 m away had its sensor at the world origin — nowhere near
/// where it is drawn, and impossible to walk into.
#[test]
fn a_parented_body_is_where_it_is_drawn() {
    let mut resources = world();
    let parent = spawn_bare(&mut resources);
    insert(
        &mut resources,
        parent,
        Transform::from_position(Vec3::new(10.0, 0.0, -20.0)),
    );
    let child = spawn_body(
        &mut resources,
        Transform::default(),
        PhysicsBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider::default(),
    );
    insert(&mut resources, child, Parent { entity: parent });

    physics_sync_system(&mut resources);

    let slot = slot_of(&resources, child).expect("the child is in the solver");
    let pose = resources
        .get::<PhysicsWorld>()
        .and_then(|world| {
            let handle = world.handle(slot)?;
            world.backend().get_transform(handle)
        })
        .expect("the child has a pose");
    assert!(
        pose.0.abs_diff_eq(Vec3::new(10.0, 0.0, -20.0), 1e-4),
        "authored at {} instead of under its parent",
        pose.0,
    );
}

/// The other half: the solver answers in world space, and a `Transform` is relative to its parent.
/// Writing the world pose straight back applied the parent's offset a second time every step.
#[test]
fn a_parented_body_writes_back_local() {
    let mut resources = world();
    let parent = spawn_bare(&mut resources);
    insert(
        &mut resources,
        parent,
        Transform::from_position(Vec3::new(100.0, 0.0, 0.0)),
    );
    let child = spawn_body(
        &mut resources,
        Transform::from_position(Vec3::new(0.0, 10.0, 0.0)),
        PhysicsBody::default(),
        Collider::default(),
    );
    insert(&mut resources, child, Parent { entity: parent });
    Playing::set(&mut resources, true);

    simulate(&mut resources, 30);

    let local = position(&resources, child);
    assert!(
        local.x.abs() < 0.01,
        "the parent's offset leaked into the child's local transform: {local}",
    );
    assert!(local.y < 9.9, "the child never fell: {local}");
}

/// Scale multiplies down the chain, and it must be the product rather than the matrix's
/// decomposition: a decomposed scale wobbles as a rotation moves, and the scale is part of the
/// body's spec — a wobbling spec rebuilds the body every frame.
#[test]
fn a_parented_scale_multiplies() {
    let mut resources = world();
    let parent = spawn_bare(&mut resources);
    insert(
        &mut resources,
        parent,
        Transform {
            scale: Vec3::splat(3.0),
            ..Default::default()
        },
    );
    let child = spawn_body(
        &mut resources,
        Transform {
            scale: Vec3::splat(2.0),
            ..Default::default()
        },
        PhysicsBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: crate::components::SHAPE_SPHERE,
            radius: 1.0,
            ..Default::default()
        },
    );
    insert(&mut resources, child, Parent { entity: parent });

    physics_sync_system(&mut resources);

    let slot = slot_of(&resources, child).expect("the child is in the solver");
    let spec = resources
        .get::<PhysicsWorld>()
        .and_then(|world| world.spec(slot))
        .expect("the slot has a spec");
    let shape = spec.resolve(None).expect("the spec resolves a shape");
    match shape {
        crate::backend::CollisionShape::Sphere { radius } => {
            assert!(
                (radius - 6.0).abs() < 1e-4,
                "radius {radius}, not 1 × 2 × 3"
            )
        }
        other => panic!("a sphere became {other:?}"),
    }
}

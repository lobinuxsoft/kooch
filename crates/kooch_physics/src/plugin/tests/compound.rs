//! Compound colliders — a body gathering shapes from its descendants (#612).

use glam::{Quat, Vec3};

use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::{Children, GlobalTransform, Parent};
use kooch_ecs::transform::Transform;

use crate::components::{Collider, KIND_DYNAMIC, PhysicsBody, SHAPE_CUBOID, SHAPE_SPHERE};
use crate::plugin::compound::{attachments_for, digest};
use crate::plugin::systems::physics_sync_system;
use crate::plugin::world::{PhysicsWorld, SolverBody};

use super::{insert, spawn_bare, spawn_body, world};

fn cuboid(half: f32) -> Collider {
    Collider {
        shape: SHAPE_CUBOID,
        half_extents: Vec3::splat(half),
        ..Default::default()
    }
}

fn sphere(radius: f32) -> Collider {
    Collider {
        shape: SHAPE_SPHERE,
        radius,
        ..Default::default()
    }
}

fn dynamic() -> PhysicsBody {
    PhysicsBody {
        kind: KIND_DYNAMIC,
        mass: 1.0,
        ..Default::default()
    }
}

/// Links `child` under `parent`, including the derived components the
/// hierarchy systems would normally maintain.
fn attach(resources: &mut kooch_core::resource::Resources, parent: Entity, child: Entity) {
    insert(resources, child, Parent { entity: parent });

    let existing = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Children>())
        .and_then(|s| s.get(parent))
        .map(|c| c.entities.clone())
        .unwrap_or_default();
    let mut entities = existing;
    entities.push(child);
    insert(resources, parent, Children { entities });
}

/// Writes the world matrices the hierarchy would derive, so the walk has
/// something to compose through.
fn set_global(resources: &mut kooch_core::resource::Resources, entity: Entity, matrix: glam::Mat4) {
    insert(resources, entity, GlobalTransform { matrix });
}

/// The behaviour the whole feature exists for: a child holding a collider
/// but no body of its own hands its shape to the body above it.
#[test]
fn a_child_collider_joins_the_parents_body() {
    let mut r = world();
    let parent = spawn_body(&mut r, Transform::default(), dynamic(), cuboid(0.5));
    set_global(&mut r, parent, glam::Mat4::IDENTITY);

    let child = spawn_bare(&mut r);
    insert(
        &mut r,
        child,
        Transform::from_position(Vec3::new(0.0, 2.0, 0.0)),
    );
    insert(&mut r, child, sphere(0.25));
    set_global(
        &mut r,
        child,
        glam::Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)),
    );
    attach(&mut r, parent, child);

    let found = attachments_for(&r, parent);
    assert_eq!(found.len(), 1, "the child contributed its shape");
    assert_eq!(
        found[0].offset,
        Vec3::new(0.0, 2.0, 0.0),
        "expressed in the body's local space",
    );

    // And the solver really receives one body carrying both.
    physics_sync_system(&mut r);
    let slot = r
        .get::<ComponentRegistry>()
        .and_then(|reg| reg.get_cpu::<SolverBody>())
        .and_then(|s| s.get(parent))
        .map(SolverBody::slot)
        .expect("the parent got a body");

    let world_res = r.get::<PhysicsWorld>().expect("physics world");
    let handle = world_res.handle(slot).expect("live handle");
    assert_eq!(
        world_res.backend().collider_count(handle),
        Some(2),
        "one body, two shapes",
    );
    assert_eq!(world_res.backend().body_count(), 1, "and only one body");
}

/// A descendant with its own body is an independent simulation. Absorbing
/// its shape would make it collide twice — once as itself, once as part of
/// its parent.
#[test]
fn a_child_with_its_own_body_is_not_absorbed() {
    let mut r = world();
    let parent = spawn_body(&mut r, Transform::default(), dynamic(), cuboid(0.5));
    set_global(&mut r, parent, glam::Mat4::IDENTITY);

    let child = spawn_body(
        &mut r,
        Transform::from_position(Vec3::new(0.0, 2.0, 0.0)),
        dynamic(),
        sphere(0.25),
    );
    set_global(
        &mut r,
        child,
        glam::Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)),
    );
    attach(&mut r, parent, child);

    assert!(
        attachments_for(&r, parent).is_empty(),
        "an independent body keeps its own shape",
    );

    physics_sync_system(&mut r);
    let world_res = r.get::<PhysicsWorld>().expect("physics world");
    assert_eq!(world_res.backend().body_count(), 2, "two separate bodies");
}

/// The walk stops at a nested body, so a grandchild belongs to *that*
/// body rather than reaching past it to the root.
#[test]
fn the_walk_stops_at_a_nested_body() {
    let mut r = world();
    let root = spawn_body(&mut r, Transform::default(), dynamic(), cuboid(0.5));
    set_global(&mut r, root, glam::Mat4::IDENTITY);

    let middle = spawn_body(&mut r, Transform::default(), dynamic(), sphere(0.25));
    set_global(&mut r, middle, glam::Mat4::IDENTITY);
    attach(&mut r, root, middle);

    let leaf = spawn_bare(&mut r);
    insert(&mut r, leaf, Transform::default());
    insert(&mut r, leaf, sphere(0.1));
    set_global(&mut r, leaf, glam::Mat4::IDENTITY);
    attach(&mut r, middle, leaf);

    assert!(
        attachments_for(&r, root).is_empty(),
        "the leaf belongs to the middle body, not the root",
    );
    assert_eq!(
        attachments_for(&r, middle).len(),
        1,
        "the middle body claims it",
    );
}

/// A grandchild with no body in between reaches the root, with its pose
/// composed through the whole chain.
#[test]
fn a_grandchild_reaches_the_body_above_it() {
    let mut r = world();
    let root = spawn_body(&mut r, Transform::default(), dynamic(), cuboid(0.5));
    set_global(&mut r, root, glam::Mat4::IDENTITY);

    let middle = spawn_bare(&mut r);
    insert(
        &mut r,
        middle,
        Transform::from_position(Vec3::new(1.0, 0.0, 0.0)),
    );
    set_global(
        &mut r,
        middle,
        glam::Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0)),
    );
    attach(&mut r, root, middle);

    let leaf = spawn_bare(&mut r);
    insert(
        &mut r,
        leaf,
        Transform::from_position(Vec3::new(0.0, 3.0, 0.0)),
    );
    insert(&mut r, leaf, sphere(0.25));
    set_global(
        &mut r,
        leaf,
        glam::Mat4::from_translation(Vec3::new(1.0, 3.0, 0.0)),
    );
    attach(&mut r, middle, leaf);

    let found = attachments_for(&r, root);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].offset,
        Vec3::new(1.0, 3.0, 0.0),
        "composed through the whole chain, not just the last hop",
    );
}

/// The digest is what makes an edited child rebuild the body. If it did
/// not change, moving a child's collider in the Inspector would leave the
/// solver simulating the old shape.
#[test]
fn moving_a_child_changes_the_digest() {
    let a = [crate::plugin::compound::Attachment {
        shape: crate::backend::CollisionShape::Sphere { radius: 1.0 },
        offset: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        material: Default::default(),
        interaction: Default::default(),
    }];
    let mut b = a;
    b[0].offset = Vec3::new(0.0, 0.1, 0.0);

    assert_ne!(digest(&a), digest(&b), "a moved shape must rebuild");
    assert_eq!(digest(&a), digest(&a), "and the digest is stable");
    assert_ne!(digest(&a), digest(&[]), "so must a removed one");
}

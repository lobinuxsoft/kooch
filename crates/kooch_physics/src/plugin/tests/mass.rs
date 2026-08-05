//! Mass properties: what a body weighs, and where its mass sits.
//!
//! #618 was filed as "a compound body's centre of mass is in the middle
//! and it looks slow". Half of that was correct physics; the other half
//! was a units bug nobody had noticed — `mass` did not mean kilograms.

use super::*;

use crate::components::SHAPE_SPHERE;

/// What the solver actually built, as opposed to what was asked for.
fn solver_mass(resources: &Resources, entity: Entity) -> f32 {
    let world = resources.get::<PhysicsWorld>().unwrap();
    let handle = world
        .handle(slot_of(resources, entity).expect("entity has no body"))
        .expect("slot is free");
    world.backend().mass(handle).expect("stale handle")
}

fn solver_center_of_mass(resources: &Resources, entity: Entity) -> Vec3 {
    let world = resources.get::<PhysicsWorld>().unwrap();
    let handle = world
        .handle(slot_of(resources, entity).expect("entity has no body"))
        .expect("slot is free");
    world
        .backend()
        .center_of_mass(handle)
        .expect("stale handle")
}

/// A body with `mass` kg and a sphere of `radius`.
fn sphere_body(resources: &mut Resources, mass: f32, radius: f32) -> Entity {
    spawn_body(
        resources,
        Transform::default(),
        PhysicsBody {
            mass,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_SPHERE,
            radius,
            ..Default::default()
        },
    )
}

/// Attaches a child carrying only a collider, at `offset` from the parent.
fn attach_child_collider(resources: &mut Resources, parent: Entity, offset: Vec3) -> Entity {
    let child = spawn_bare(resources);
    insert(resources, child, Transform::from_position(offset));
    insert(resources, child, Collider::default());
    insert(
        resources,
        child,
        kooch_ecs::hierarchy::Parent { entity: parent },
    );
    insert(
        resources,
        child,
        kooch_ecs::hierarchy::GlobalTransform {
            matrix: glam::Mat4::from_translation(offset),
        },
    );
    // The compound walk reads Children, which the hierarchy system
    // normally maintains; this harness has no hierarchy system.
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<kooch_ecs::hierarchy::Children>()
        && let Some(children) = storage.get_mut(parent)
    {
        children.entities.push(child);
        return child;
    }
    insert(
        resources,
        parent,
        kooch_ecs::hierarchy::Children {
            entities: vec![child],
        },
    );
    insert(
        resources,
        parent,
        kooch_ecs::hierarchy::GlobalTransform {
            matrix: glam::Mat4::IDENTITY,
        },
    );
    child
}

/// The units bug at the heart of #618: rapier's `additional_mass` is
/// *added* to the mass a collider's volume implies, so the authored number
/// meant a different weight for every shape. A two-metre sphere authored
/// at 1 kg weighed thirty-four.
#[test]
fn a_body_weighs_exactly_what_was_authored_whatever_its_shape() {
    for radius in [0.1, 0.5, 2.0] {
        let mut resources = world();
        let entity = sphere_body(&mut resources, 3.0, radius);
        physics_sync_system(&mut resources);

        let mass = solver_mass(&resources, entity);
        assert!(
            (mass - 3.0).abs() < 1e-3,
            "a 3 kg body with a {radius} m sphere weighs {mass} kg",
        );
    }
}

/// The other half of #618: adding shapes made the body heavier, and
/// nothing in the Inspector said so.
#[test]
fn inherited_shapes_add_collision_and_no_mass() {
    let mut resources = world();
    let parent = sphere_body(&mut resources, 3.0, 0.5);
    physics_sync_system(&mut resources);
    let alone = solver_mass(&resources, parent);

    attach_child_collider(&mut resources, parent, Vec3::new(4.0, 0.0, 0.0));
    physics_sync_system(&mut resources);

    assert!(
        (solver_mass(&resources, parent) - alone).abs() < 1e-4,
        "a child collider changed the body's mass from {alone} to {}",
        solver_mass(&resources, parent),
    );
    // And the shape did arrive — otherwise this test passes for the wrong
    // reason, by the attachment never happening at all.
    let world_ref = resources.get::<PhysicsWorld>().unwrap();
    let handle = world_ref
        .handle(slot_of(&resources, parent).expect("no body"))
        .expect("slot is free");
    assert_eq!(
        world_ref.backend().collider_count(handle),
        Some(2),
        "the child's shape never reached the body",
    );
}

/// What the author actually reported: the centre of mass drifting towards
/// shapes they thought of as collision only.
#[test]
fn inherited_shapes_do_not_move_the_centre_of_mass() {
    let mut resources = world();
    let parent = sphere_body(&mut resources, 3.0, 0.5);
    attach_child_collider(&mut resources, parent, Vec3::new(4.0, 0.0, 0.0));
    physics_sync_system(&mut resources);

    let center = solver_center_of_mass(&resources, parent);
    assert!(
        center.length() < 1e-3,
        "a child four metres away pulled the centre of mass to {center}",
    );
}

/// A vehicle wants its centre of mass low, and no arrangement of collision
/// shapes says that as directly.
#[test]
fn an_explicit_centre_of_mass_is_honoured() {
    let mut resources = world();
    let entity = spawn_body(
        &mut resources,
        Transform::default(),
        PhysicsBody {
            mass: 3.0,
            center_of_mass_enabled: true,
            center_of_mass: Vec3::new(0.0, -0.4, 0.0),
            ..Default::default()
        },
        Collider::default(),
    );
    physics_sync_system(&mut resources);

    let center = solver_center_of_mass(&resources, entity);
    assert!(
        center.abs_diff_eq(Vec3::new(0.0, -0.4, 0.0), 1e-4),
        "the authored centre of mass became {center}",
    );
}

/// `density` is authoring input for the Inspector's Calculate mass button
/// and nothing else reads it. If it reached the spec, editing it would
/// retire the body — dropping its velocity mid-play for a number the
/// solver never sees.
#[test]
fn editing_the_density_does_not_rebuild_the_body() {
    let mut resources = world();
    let entity = sphere_body(&mut resources, 3.0, 0.5);
    physics_sync_system(&mut resources);
    let slot = slot_of(&resources, entity);

    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<PhysicsBody>()
        && let Some(body) = storage.get_mut(entity)
    {
        body.density = 7850.0;
    }
    physics_sync_system(&mut resources);

    assert_eq!(
        slot_of(&resources, entity),
        slot,
        "a density edit churned the body",
    );
    assert!((solver_mass(&resources, entity) - 3.0).abs() < 1e-3);
}

/// Editing the centre of mass *must* rebuild — rapier bakes mass
/// properties into the body, the same as shapes.
#[test]
fn editing_the_centre_of_mass_reaches_the_solver() {
    let mut resources = world();
    let entity = sphere_body(&mut resources, 3.0, 0.5);
    physics_sync_system(&mut resources);

    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<PhysicsBody>()
        && let Some(body) = storage.get_mut(entity)
    {
        body.center_of_mass_enabled = true;
        body.center_of_mass = Vec3::new(0.2, 0.0, 0.0);
    }
    physics_sync_system(&mut resources);

    assert!(
        solver_center_of_mass(&resources, entity).abs_diff_eq(Vec3::new(0.2, 0.0, 0.0), 1e-4),
        "the edit never reached the solver",
    );
}

/// A mass field mid-edit passes through zero, and a body with no inertia
/// takes infinite angular acceleration from any torque. The NaNs outlive
/// the typo.
#[test]
fn a_zero_mass_is_clamped_rather_than_producing_nans() {
    let mut resources = world();
    let entity = sphere_body(&mut resources, 0.0, 0.5);
    physics_sync_system(&mut resources);

    let mass = solver_mass(&resources, entity);
    assert!(mass > 0.0, "a zero-mass dynamic body reached the solver");
    assert!(mass.is_finite(), "mass is {mass}");
    assert!(
        solver_center_of_mass(&resources, entity).is_finite(),
        "the centre of mass went non-finite",
    );

    Playing::set(&mut resources, true);
    simulate(&mut resources, 30);
    assert!(
        position(&resources, entity).is_finite(),
        "the body left the number line",
    );
}

/// The mass properties have to be right *before* anything steps: the
/// editor authors a world it does not simulate, and #563 will draw the
/// centre of mass there.
#[test]
fn mass_properties_are_current_without_stepping() {
    let mut resources = world();
    let entity = sphere_body(&mut resources, 12.5, 0.5);

    physics_sync_system(&mut resources);

    assert!(
        (solver_mass(&resources, entity) - 12.5).abs() < 1e-3,
        "mass reads {} before the first step",
        solver_mass(&resources, entity),
    );
}

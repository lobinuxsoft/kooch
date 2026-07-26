//! The debug overlay against a real world, not a mocked one.
//!
//! The unit tests next to the adapter cover the colour maths and the flag
//! mapping. These check the thing that actually matters: that asking the
//! solver to describe itself produces geometry where the solver has
//! something to say, and nothing where it does not.

use super::*;

use crate::backend::{DebugCategories, DebugLine};
use crate::components::{JOINT_FIXED, Joint};

fn lines(resources: &Resources, categories: DebugCategories) -> Vec<DebugLine> {
    let mut out = Vec::new();
    resources
        .get::<PhysicsWorld>()
        .unwrap()
        .backend()
        .debug_lines(categories, &mut out);
    out
}

fn ground(resources: &mut Resources) -> Entity {
    spawn_body(
        resources,
        Transform::from_position(Vec3::new(0.0, -1.0, 0.0)),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(10.0, 1.0, 10.0),
            ..Default::default()
        },
    )
}

/// "The overlay costs nothing when disabled" has to mean the walk never
/// happens, not that it happens and returns an empty vector.
#[test]
fn nothing_is_produced_when_every_category_is_off() {
    let mut resources = world();
    falling_sphere(&mut resources, 5.0);
    ground(&mut resources);
    physics_sync_system(&mut resources);

    assert!(lines(&resources, DebugCategories::default()).is_empty());
}

/// An empty world has nothing to describe, however much is switched on.
#[test]
fn an_empty_world_produces_nothing() {
    let resources = world();
    assert!(lines(&resources, DebugCategories::all()).is_empty());
}

#[test]
fn collider_shapes_produce_geometry() {
    let mut resources = world();
    falling_sphere(&mut resources, 5.0);
    physics_sync_system(&mut resources);

    let lines = lines(
        &resources,
        DebugCategories {
            collider_shapes: true,
            ..Default::default()
        },
    );
    assert!(!lines.is_empty(), "a sphere produced no outline");
    assert!(
        lines
            .iter()
            .all(|l| l.start.is_finite() && l.end.is_finite()),
        "the walk produced a non-finite point",
    );
}

/// The category the whole issue is really about: where two bodies are
/// actually touching is not derivable from any component.
#[test]
fn contacts_appear_only_once_bodies_touch() {
    let mut resources = world();
    let sphere = falling_sphere(&mut resources, 1.0);
    ground(&mut resources);
    let categories = DebugCategories {
        contacts: true,
        ..Default::default()
    };

    physics_sync_system(&mut resources);
    assert!(
        lines(&resources, categories).is_empty(),
        "a body in mid-air is already reporting contacts",
    );

    Playing::set(&mut resources, true);
    simulate(&mut resources, 120);

    assert!(
        position(&resources, sphere).y < 1.0,
        "setup: the sphere never fell to the ground",
    );
    assert!(
        !lines(&resources, categories).is_empty(),
        "two bodies resting on each other report no contact",
    );
}

/// Joints are new and completely invisible otherwise — a hinge anchored
/// to the wrong point looks exactly like one that is not working.
#[test]
fn joints_produce_geometry() {
    let mut resources = world();
    let anchor = ground(&mut resources);
    let hanging = falling_sphere(&mut resources, 2.0);
    let joint = spawn_bare(&mut resources);
    insert(
        &mut resources,
        joint,
        Joint {
            kind: JOINT_FIXED,
            body_a: anchor,
            body_b: hanging,
            ..Default::default()
        },
    );
    physics_sync_system(&mut resources);

    assert!(
        !lines(
            &resources,
            DebugCategories {
                joints: true,
                ..Default::default()
            }
        )
        .is_empty(),
        "a live joint produced no geometry",
    );
}

/// Body axes are drawn at the centre of mass, which is the quantity that
/// made #618 impossible to diagnose by looking at the viewport.
#[test]
fn body_axes_are_drawn_at_the_centre_of_mass() {
    let mut resources = world();
    // A centre of mass deliberately away from the entity's origin, so
    // "drawn at the origin" and "drawn at the centre of mass" differ.
    let offset = Vec3::new(0.0, -0.4, 0.0);
    spawn_body(
        &mut resources,
        Transform::default(),
        RigidBody {
            mass: 1.0,
            center_of_mass_enabled: true,
            center_of_mass: offset,
            ..Default::default()
        },
        Collider::default(),
    );
    physics_sync_system(&mut resources);

    let lines = lines(
        &resources,
        DebugCategories {
            body_axes: true,
            ..Default::default()
        },
    );
    assert!(!lines.is_empty(), "no axes were drawn");
    // Rapier draws the axes as three segments meeting at the centre of
    // mass, so that point is the one every segment shares.
    assert!(
        lines.iter().any(|l| l.start.abs_diff_eq(offset, 1e-3)),
        "the axes are not at the centre of mass; first start is {:?}",
        lines.first().map(|l| l.start),
    );
}

/// Each switch has to add geometry on its own. A category that only draws
/// when another one happens to be on is a box that lies.
#[test]
fn each_category_contributes_on_its_own() {
    let mut resources = world();
    let anchor = ground(&mut resources);
    let hanging = falling_sphere(&mut resources, 2.0);
    let joint = spawn_bare(&mut resources);
    insert(
        &mut resources,
        joint,
        Joint {
            kind: JOINT_FIXED,
            body_a: anchor,
            body_b: hanging,
            ..Default::default()
        },
    );
    physics_sync_system(&mut resources);

    for (name, categories) in [
        (
            "collider_shapes",
            DebugCategories {
                collider_shapes: true,
                ..Default::default()
            },
        ),
        (
            "joints",
            DebugCategories {
                joints: true,
                ..Default::default()
            },
        ),
        (
            "collider_aabbs",
            DebugCategories {
                collider_aabbs: true,
                ..Default::default()
            },
        ),
        (
            "body_axes",
            DebugCategories {
                body_axes: true,
                ..Default::default()
            },
        ),
    ] {
        assert!(
            !lines(&resources, categories).is_empty(),
            "{name} on its own drew nothing",
        );
    }
}

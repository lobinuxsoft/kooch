//! Joints: the issue's acceptance list, plus the lifetime cases the sync
//! pass has to get right for a joint not to outlive its bodies.

use super::*;

use crate::components::{JOINT_FIXED, JOINT_REVOLUTE, JOINT_SPHERICAL, Joint, MOTOR_ACCELERATION};

/// Spawns an entity carrying only a joint, as an author would: the joint
/// lives on its own entity so a body can be in as many as it likes.
fn spawn_joint(resources: &mut Resources, joint: Joint) -> Entity {
    let entity = spawn_bare(resources);
    insert(resources, entity, joint);
    entity
}

/// An immovable cuboid, the thing a joint hangs off.
fn anchor(resources: &mut Resources, position: Vec3) -> Entity {
    spawn_body(
        resources,
        Transform::from_position(position),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(0.25),
            ..Default::default()
        },
    )
}

/// A one-kilo dynamic cuboid.
fn part(resources: &mut Resources, position: Vec3) -> Entity {
    spawn_body(
        resources,
        Transform::from_position(position),
        RigidBody::default(),
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(0.25),
            ..Default::default()
        },
    )
}

fn joint_count(resources: &Resources) -> usize {
    resources
        .get::<PhysicsWorld>()
        .unwrap()
        .backend()
        .joint_count()
}

fn is_built(resources: &Resources, joint: Entity) -> bool {
    resources
        .get::<PhysicsWorld>()
        .unwrap()
        .joints()
        .is_built(joint)
}

/// How far an entity has rotated from where it started.
fn rotation_angle(resources: &Resources, entity: Entity) -> f32 {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(entity))
        .map(|t| t.rotation.angle_between(Quat::IDENTITY))
        .expect("entity has no Transform")
}

fn play(resources: &mut Resources) {
    Playing::set(resources, true);
}

// ---------------------------------------------------------------------------
// Building, waiting, rebuilding
// ---------------------------------------------------------------------------

#[test]
fn a_joint_naming_two_bodies_reaches_the_solver() {
    let mut resources = world();
    let a = anchor(&mut resources, Vec3::ZERO);
    let b = part(&mut resources, Vec3::new(1.0, 0.0, 0.0));
    let joint = spawn_joint(
        &mut resources,
        Joint {
            kind: JOINT_FIXED,
            body_a: a,
            body_b: b,
            ..Default::default()
        },
    );

    physics_sync_system(&mut resources);

    assert_eq!(joint_count(&resources), 1);
    assert!(is_built(&resources, joint));
}

/// Acceptance: "A joint whose partner is not yet spawned must wait rather
/// than panic or drop." Under streaming this is the normal state, not an
/// error, so the joint has to survive to be built later.
#[test]
fn a_joint_waits_for_a_body_that_does_not_exist_yet() {
    let mut resources = world();
    let a = anchor(&mut resources, Vec3::ZERO);
    let b = spawn_bare(&mut resources);
    let joint = spawn_joint(
        &mut resources,
        Joint {
            body_a: a,
            body_b: b,
            ..Default::default()
        },
    );

    physics_sync_system(&mut resources);
    assert_eq!(joint_count(&resources), 0, "built against a missing body");
    assert!(!is_built(&resources, joint));

    // The partner arrives, as it would when its cell streams in.
    insert(&mut resources, b, Transform::from_position(Vec3::X));
    insert(&mut resources, b, RigidBody::default());
    insert(&mut resources, b, Collider::default());
    physics_sync_system(&mut resources);

    assert_eq!(joint_count(&resources), 1, "the joint never caught up");
    assert!(is_built(&resources, joint));
}

/// Sync runs every frame; an unchanged world must not churn its joints.
#[test]
fn syncing_repeatedly_does_not_duplicate_joints() {
    let mut resources = world();
    let a = anchor(&mut resources, Vec3::ZERO);
    let b = part(&mut resources, Vec3::X);
    spawn_joint(
        &mut resources,
        Joint {
            body_a: a,
            body_b: b,
            ..Default::default()
        },
    );

    for _ in 0..3 {
        physics_sync_system(&mut resources);
    }

    assert_eq!(joint_count(&resources), 1);
}

/// An Inspector edit has to reach the solver, and rapier bakes a joint's
/// parameters at build time — so the joint is rebuilt, exactly as a
/// collider edit rebuilds a body.
#[test]
fn editing_a_joint_rebuilds_it() {
    let mut resources = world();
    let a = anchor(&mut resources, Vec3::ZERO);
    let b = part(&mut resources, Vec3::X);
    let joint = spawn_joint(
        &mut resources,
        Joint {
            body_a: a,
            body_b: b,
            ..Default::default()
        },
    );
    physics_sync_system(&mut resources);

    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Joint>()
        && let Some(spec) = storage.get_mut(joint)
    {
        spec.kind = JOINT_REVOLUTE;
    }
    physics_sync_system(&mut resources);

    assert_eq!(joint_count(&resources), 1, "the old joint leaked");
    assert!(is_built(&resources, joint));
}

/// Acceptance: "Despawning either entity removes the joint cleanly."
#[test]
fn losing_a_body_removes_the_joint() {
    let mut resources = world();
    let a = anchor(&mut resources, Vec3::ZERO);
    let b = part(&mut resources, Vec3::X);
    let joint = spawn_joint(
        &mut resources,
        Joint {
            body_a: a,
            body_b: b,
            ..Default::default()
        },
    );
    physics_sync_system(&mut resources);
    assert_eq!(joint_count(&resources), 1);

    remove::<RigidBody>(&mut resources, b);
    physics_sync_system(&mut resources);

    assert_eq!(joint_count(&resources), 0, "the joint outlived its body");
    assert!(!is_built(&resources, joint));
}

#[test]
fn losing_the_joint_component_removes_the_joint() {
    let mut resources = world();
    let a = anchor(&mut resources, Vec3::ZERO);
    let b = part(&mut resources, Vec3::X);
    let joint = spawn_joint(
        &mut resources,
        Joint {
            body_a: a,
            body_b: b,
            ..Default::default()
        },
    );
    physics_sync_system(&mut resources);

    remove::<Joint>(&mut resources, joint);
    physics_sync_system(&mut resources);

    assert_eq!(joint_count(&resources), 0);
}

/// A joint from a body to itself is a degenerate system, not a constraint.
#[test]
fn a_joint_cannot_constrain_a_body_to_itself() {
    let mut resources = world();
    let a = part(&mut resources, Vec3::ZERO);
    let joint = spawn_joint(
        &mut resources,
        Joint {
            body_a: a,
            body_b: a,
            ..Default::default()
        },
    );

    physics_sync_system(&mut resources);

    assert_eq!(joint_count(&resources), 0);
    assert!(!is_built(&resources, joint));
}

// ---------------------------------------------------------------------------
// Simulation
// ---------------------------------------------------------------------------

/// A fixed joint is the baseline: without one the part falls, with one it
/// hangs. Everything below assumes joints actually constrain.
#[test]
fn a_fixed_joint_holds_a_body_up_against_gravity() {
    let mut resources = world();
    let a = anchor(&mut resources, Vec3::new(0.0, 5.0, 0.0));
    let b = part(&mut resources, Vec3::new(1.0, 5.0, 0.0));
    spawn_joint(
        &mut resources,
        Joint {
            kind: JOINT_FIXED,
            body_a: a,
            body_b: b,
            anchor_a: Vec3::new(1.0, 0.0, 0.0),
            ..Default::default()
        },
    );

    play(&mut resources);
    simulate(&mut resources, 120);

    let y = position(&resources, b).y;
    assert!(
        y > 4.0,
        "a welded body fell to {y}; the joint is not holding",
    );
}

/// Acceptance: "A door hinged with a revolute joint swings and stops at its
/// limits."
///
/// Asserted against an unlimited control rather than against rapier's
/// zero-angle convention: what the author cares about is that the limit
/// stops the door sooner than no limit would, and that it stops near the
/// value they typed.
#[test]
fn a_hinged_door_swings_and_stops_at_its_limit() {
    fn swing(limits: Option<f32>) -> f32 {
        let mut resources = world();
        let frame = anchor(&mut resources, Vec3::new(0.0, 5.0, 0.0));
        let door = part(&mut resources, Vec3::new(1.5, 5.0, 0.0));
        spawn_joint(
            &mut resources,
            Joint {
                kind: JOINT_REVOLUTE,
                body_a: frame,
                body_b: door,
                // A horizontal hinge, so gravity has a torque about it and
                // the door actually swings.
                axis: Vec3::Z,
                // Both anchors land on the same world point, half a metre
                // from the door's centre of mass — a hinge through the
                // centre has no lever arm, and the door would just sit
                // there however free the joint is.
                anchor_a: Vec3::new(1.0, 0.0, 0.0),
                anchor_b: Vec3::new(-0.5, 0.0, 0.0),
                limits_enabled: limits.is_some(),
                limit_min: -limits.unwrap_or(0.0),
                limit_max: 0.0,
                ..Default::default()
            },
        );
        play(&mut resources);
        simulate(&mut resources, 240);
        rotation_angle(&resources, door)
    }

    let limited = swing(Some(0.4));
    let free = swing(None);

    assert!(
        free > 0.8,
        "the control door barely moved ({free} rad); the hinge is not free",
    );
    assert!(
        limited < 0.55,
        "the limited door reached {limited} rad, well past its 0.4 rad limit",
    );
    assert!(
        limited < free,
        "limiting the hinge changed nothing ({limited} vs {free} rad)",
    );
}

/// Acceptance: "A rope of spherical joints hangs and swings without
/// stretching visibly."
#[test]
fn a_chain_of_spherical_joints_does_not_stretch() {
    const LINKS: usize = 4;
    const SPACING: f32 = 1.0;

    let mut resources = world();
    let top = anchor(&mut resources, Vec3::new(0.0, 10.0, 0.0));
    let mut chain = vec![top];
    for index in 1..=LINKS {
        chain.push(part(
            &mut resources,
            Vec3::new(0.0, 10.0 - SPACING * index as f32, 0.0),
        ));
    }
    for pair in chain.windows(2) {
        spawn_joint(
            &mut resources,
            Joint {
                kind: JOINT_SPHERICAL,
                body_a: pair[0],
                body_b: pair[1],
                anchor_a: Vec3::new(0.0, -SPACING / 2.0, 0.0),
                anchor_b: Vec3::new(0.0, SPACING / 2.0, 0.0),
                ..Default::default()
            },
        );
    }

    play(&mut resources);
    simulate(&mut resources, 180);

    for (index, pair) in chain.windows(2).enumerate() {
        let separation = position(&resources, pair[1]).distance(position(&resources, pair[0]));
        assert!(
            (separation - SPACING).abs() < 0.15,
            "link {index} stretched to {separation} m from {SPACING} m",
        );
    }
    let bottom = position(&resources, *chain.last().unwrap());
    assert!(
        bottom.y > 10.0 - SPACING * LINKS as f32 - 0.5,
        "the chain fell away entirely, ending at {bottom}",
    );
}

/// Acceptance: "A motorised revolute joint drives a wheel at a target
/// velocity."
#[test]
fn a_motorised_hinge_drives_a_wheel() {
    fn spin(motorised: bool) -> f32 {
        let mut resources = world();
        let hub = anchor(&mut resources, Vec3::new(0.0, 5.0, 0.0));
        let wheel = part(&mut resources, Vec3::new(0.0, 5.0, 0.0));
        spawn_joint(
            &mut resources,
            Joint {
                kind: JOINT_REVOLUTE,
                body_a: hub,
                body_b: wheel,
                axis: Vec3::Y,
                motor_enabled: true,
                motor_model: MOTOR_ACCELERATION,
                motor_target_velocity: if motorised { 5.0 } else { 0.0 },
                // Stiff enough to reach the target inside the window this
                // test measures; a soft motor is a different assertion.
                motor_damping: if motorised { 20.0 } else { 0.0 },
                ..Default::default()
            },
        );
        play(&mut resources);
        // Half a second at 5 rad/s is 2.5 rad — under pi, so the measured
        // angle has not wrapped and "more" still means more.
        simulate(&mut resources, 30);
        rotation_angle(&resources, wheel)
    }

    let driven = spin(true);
    let passive = spin(false);

    assert!(
        driven > 1.5,
        "the motor turned the wheel only {driven} rad in half a second",
    );
    assert!(
        passive < 0.05,
        "the unmotorised wheel turned {passive} rad on its own",
    );
}

/// Acceptance: "Breaking a joint above its threshold detaches the body."
///
/// A 1 kg body under gravity loads its joint with about 0.16 N·s per 60 Hz
/// step, so a threshold well under that breaks on the first loaded step.
#[test]
fn a_joint_breaks_above_its_threshold_and_stays_broken() {
    let mut resources = world();
    let hook = anchor(&mut resources, Vec3::new(0.0, 10.0, 0.0));
    let load = part(&mut resources, Vec3::new(0.0, 9.0, 0.0));
    let joint = spawn_joint(
        &mut resources,
        Joint {
            kind: JOINT_FIXED,
            body_a: hook,
            body_b: load,
            anchor_a: Vec3::new(0.0, -1.0, 0.0),
            breakable: true,
            break_impulse: 0.02,
            ..Default::default()
        },
    );

    play(&mut resources);
    simulate(&mut resources, 60);

    assert!(
        !is_built(&resources, joint),
        "the joint held past its limit"
    );
    assert_eq!(joint_count(&resources), 0, "the broken joint leaked");
    let y = position(&resources, load).y;
    assert!(y < 8.0, "the detached body did not fall; it is at {y}");

    // The component is still authored. Rebuilding it here would break it
    // again next step, forever.
    simulate(&mut resources, 60);
    assert!(!is_built(&resources, joint), "a broken joint came back");
    assert_eq!(joint_count(&resources), 0);
}

/// The same load under a threshold it cannot reach must not break — a
/// breaking test that passes for both thresholds proves nothing.
#[test]
fn a_joint_below_its_threshold_holds() {
    let mut resources = world();
    let hook = anchor(&mut resources, Vec3::new(0.0, 10.0, 0.0));
    let load = part(&mut resources, Vec3::new(0.0, 9.0, 0.0));
    let joint = spawn_joint(
        &mut resources,
        Joint {
            kind: JOINT_FIXED,
            body_a: hook,
            body_b: load,
            anchor_a: Vec3::new(0.0, -1.0, 0.0),
            breakable: true,
            break_impulse: 1000.0,
            ..Default::default()
        },
    );

    play(&mut resources);
    simulate(&mut resources, 120);

    assert!(
        is_built(&resources, joint),
        "the joint broke under its rating"
    );
    assert!(position(&resources, load).y > 8.0, "the load fell anyway");
}

/// Stopping drops every `PhysicsBody`, so the world is rebuilt from the
/// restored ECS. The joints have to come back with it — including one that
/// broke during the session, which is the whole reason the registry keys on
/// body handles rather than on a flag it would have to clear by hand.
#[test]
fn stopping_rebuilds_a_joint_that_broke_while_playing() {
    let mut resources = world();
    let hook = anchor(&mut resources, Vec3::new(0.0, 10.0, 0.0));
    let load = part(&mut resources, Vec3::new(0.0, 9.0, 0.0));
    let joint = spawn_joint(
        &mut resources,
        Joint {
            kind: JOINT_FIXED,
            body_a: hook,
            body_b: load,
            anchor_a: Vec3::new(0.0, -1.0, 0.0),
            breakable: true,
            break_impulse: 0.02,
            ..Default::default()
        },
    );

    play(&mut resources);
    simulate(&mut resources, 60);
    assert!(!is_built(&resources, joint), "setup: the joint never broke");

    // What stop does: the runtime components go, and the next sync builds
    // the world again from what the ECS still holds.
    Playing::set(&mut resources, false);
    remove::<PhysicsBody>(&mut resources, hook);
    remove::<PhysicsBody>(&mut resources, load);
    physics_sync_system(&mut resources);

    assert!(
        is_built(&resources, joint),
        "the joint stayed broken across a stop",
    );
}

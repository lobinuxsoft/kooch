//! The solver reporting back: the issue's acceptance list.

use super::*;

use ome_core::event::Events;
use ome_ecs::reflect::EntityRef;

use crate::components::{JOINT_FIXED, Joint};
use crate::plugin::{CollisionStarted, CollisionStopped, ContactForce, JointBroke};

/// The harness builds `Resources` by hand, so the event buffers the plugin
/// would have registered have to be inserted here.
fn listening_world() -> Resources {
    let mut resources = world();
    resources.insert(Events::<CollisionStarted>::default());
    resources.insert(Events::<CollisionStopped>::default());
    resources.insert(Events::<ContactForce>::default());
    resources.insert(Events::<JointBroke>::default());
    resources
}

/// Runs a frame the way the plugin schedules one: lifecycle, sync, step,
/// writeback, drain.
fn frame(resources: &mut Resources) {
    crate::plugin::events::physics_lifecycle_system(resources);
    physics_sync_system(resources);
    if Playing::is_playing(resources) {
        physics_step_system(resources);
        physics_writeback_system(resources);
        crate::plugin::events::drain_physics_events(resources);
    }
    // What the runner does between frames: yesterday's events become
    // readable, and the ones read last frame go.
    for _ in 0..1 {
        if let Some(events) = resources.get_mut::<Events<CollisionStarted>>() {
            events.update();
        }
        if let Some(events) = resources.get_mut::<Events<CollisionStopped>>() {
            events.update();
        }
        if let Some(events) = resources.get_mut::<Events<ContactForce>>() {
            events.update();
        }
        if let Some(events) = resources.get_mut::<Events<JointBroke>>() {
            events.update();
        }
    }
}

fn collected<E: Send + Sync + Copy + 'static>(resources: &Resources) -> Vec<E> {
    resources
        .get::<Events<E>>()
        .map(|events| events.read().copied().collect())
        .unwrap_or_default()
}

/// Runs `frames`, accumulating every event of one type as it appears.
fn run_collecting<E: Send + Sync + Copy + 'static>(
    resources: &mut Resources,
    frames: u32,
) -> Vec<E> {
    let mut seen = Vec::new();
    for _ in 0..frames {
        frame(resources);
        seen.extend(collected::<E>(resources));
    }
    seen
}

fn ground(resources: &mut Resources, half_y: f32) -> Entity {
    spawn_body(
        resources,
        Transform::from_position(Vec3::new(0.0, -half_y, 0.0)),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(50.0, half_y, 50.0),
            ..Default::default()
        },
    )
}

// ---------------------------------------------------------------------------
// Sensors
// ---------------------------------------------------------------------------

/// Acceptance: "a sensor volume reports enter and exit with the right
/// entity."
///
/// The entity half is asserted here; enter-and-exit as a pair is the next
/// test, which watches one body fall the whole way through.
#[test]
fn a_sensor_reports_the_right_entities() {
    let mut resources = listening_world();
    let trigger = spawn_body(
        &mut resources,
        Transform::from_position(Vec3::new(0.0, 5.0, 0.0)),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(2.0, 0.5, 2.0),
            sensor: true,
            collision_events: true,
            ..Default::default()
        },
    );
    let faller = falling_sphere(&mut resources, 8.0);
    Playing::set(&mut resources, true);

    let started = run_collecting::<CollisionStarted>(&mut resources, 240);
    let enter = started
        .iter()
        .find(|e| (e.a == trigger && e.b == faller) || (e.a == faller && e.b == trigger))
        .unwrap_or_else(|| {
            panic!(
                "the sensor reported {} events, none naming both entities",
                started.len()
            )
        });
    assert!(
        enter.sensor,
        "a sensor overlap was reported as a solid contact",
    );
}

/// Falling *through* a sensor has to produce both halves. Asserted in one
/// run so enter and exit are the same pass of the same body.
#[test]
fn falling_through_a_sensor_reports_both_halves() {
    let mut resources = listening_world();
    spawn_body(
        &mut resources,
        Transform::from_position(Vec3::new(0.0, 5.0, 0.0)),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(4.0, 0.5, 4.0),
            sensor: true,
            collision_events: true,
            ..Default::default()
        },
    );
    falling_sphere(&mut resources, 8.0);
    Playing::set(&mut resources, true);

    let mut entered = 0;
    let mut exited = 0;
    for _ in 0..300 {
        frame(&mut resources);
        entered += collected::<CollisionStarted>(&resources).len();
        exited += collected::<CollisionStopped>(&resources).len();
    }

    assert!(entered > 0, "the body never entered the sensor");
    assert!(exited > 0, "the body entered and never left");
}

/// A sensor must not push. If it solved contacts it would be a floor, and
/// the body would stop on it instead of passing through.
#[test]
fn a_sensor_does_not_stop_anything() {
    let mut resources = listening_world();
    spawn_body(
        &mut resources,
        Transform::from_position(Vec3::new(0.0, 3.0, 0.0)),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(4.0, 0.5, 4.0),
            sensor: true,
            collision_events: true,
            ..Default::default()
        },
    );
    let faller = falling_sphere(&mut resources, 6.0);
    Playing::set(&mut resources, true);

    for _ in 0..180 {
        frame(&mut resources);
    }

    assert!(
        position(&resources, faller).y < 2.0,
        "the sensor stopped the body at {}",
        position(&resources, faller).y,
    );
}

/// Events are opt-in. A collider that did not ask must produce nothing,
/// which is what keeps the cost proportional to what a game listens for.
#[test]
fn a_collider_that_did_not_ask_reports_nothing() {
    let mut resources = listening_world();
    ground(&mut resources, 1.0);
    falling_sphere(&mut resources, 2.0);
    Playing::set(&mut resources, true);

    let started = run_collecting::<CollisionStarted>(&mut resources, 180);
    assert!(
        started.is_empty(),
        "{} events from colliders that never opted in",
        started.len(),
    );
}

// ---------------------------------------------------------------------------
// Contact force
// ---------------------------------------------------------------------------

/// Acceptance: "a hard impact raises a contact-force event; a gentle touch
/// does not."
#[test]
fn only_a_hard_enough_impact_raises_a_force_event() {
    fn impacts(drop_height: f32, threshold: f32) -> usize {
        let mut resources = listening_world();
        spawn_body(
            &mut resources,
            Transform::from_position(Vec3::new(0.0, -1.0, 0.0)),
            RigidBody {
                kind: KIND_STATIC,
                mass: 0.0,
                ..Default::default()
            },
            Collider {
                shape: SHAPE_CUBOID,
                half_extents: Vec3::new(50.0, 1.0, 50.0),
                contact_force_events: true,
                contact_force_threshold: threshold,
                ..Default::default()
            },
        );
        spawn_body(
            &mut resources,
            Transform::from_position(Vec3::new(0.0, drop_height, 0.0)),
            RigidBody {
                mass: 20.0,
                ..Default::default()
            },
            Collider::default(),
        );
        Playing::set(&mut resources, true);
        run_collecting::<ContactForce>(&mut resources, 240).len()
    }

    // A tall drop against a threshold it clears, and a body already resting
    // against a threshold far above what its weight produces.
    let hard = impacts(12.0, 1.0);
    let gentle = impacts(0.55, 10_000.0);

    assert!(hard > 0, "a 20 kg body dropped 12 m raised no force event");
    assert_eq!(
        gentle, 0,
        "a body resting on the floor raised {gentle} events over a huge threshold",
    );
}

/// The peak can never exceed the total, and a listener told otherwise
/// cannot tell a spread blow from a spike.
#[test]
fn a_force_event_carries_a_sane_total_and_peak() {
    let mut resources = listening_world();
    spawn_body(
        &mut resources,
        Transform::from_position(Vec3::new(0.0, -1.0, 0.0)),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(50.0, 1.0, 50.0),
            contact_force_events: true,
            contact_force_threshold: 1.0,
            ..Default::default()
        },
    );
    spawn_body(
        &mut resources,
        Transform::from_position(Vec3::new(0.0, 12.0, 0.0)),
        RigidBody {
            mass: 20.0,
            ..Default::default()
        },
        Collider::default(),
    );
    Playing::set(&mut resources, true);

    let events = run_collecting::<ContactForce>(&mut resources, 240);
    let event = events.first().expect("no force event to inspect");
    assert!(event.total_force_magnitude > 0.0);
    assert!(
        event.max_force_magnitude <= event.total_force_magnitude + 1e-3,
        "peak {} exceeds total {}",
        event.max_force_magnitude,
        event.total_force_magnitude,
    );
}

// ---------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------

/// Acceptance: "two bodies in non-overlapping collision groups pass through
/// each other."
#[test]
fn disjoint_collision_groups_pass_through() {
    fn fell_through(groups: (u32, u32)) -> bool {
        let mut resources = listening_world();
        spawn_body(
            &mut resources,
            Transform::from_position(Vec3::new(0.0, 0.0, 0.0)),
            RigidBody {
                kind: KIND_STATIC,
                mass: 0.0,
                ..Default::default()
            },
            Collider {
                shape: SHAPE_CUBOID,
                half_extents: Vec3::new(50.0, 0.5, 50.0),
                collision_memberships: groups.0,
                collision_filter: groups.0,
                ..Default::default()
            },
        );
        let faller = spawn_body(
            &mut resources,
            Transform::from_position(Vec3::new(0.0, 4.0, 0.0)),
            RigidBody::default(),
            Collider {
                collision_memberships: groups.1,
                collision_filter: groups.1,
                ..Default::default()
            },
        );
        Playing::set(&mut resources, true);
        for _ in 0..180 {
            frame(&mut resources);
        }
        position(&resources, faller).y < -2.0
    }

    assert!(
        !fell_through((0b0001, 0b0001)), // same group: the floor holds
        "a body in the floor's own group fell through it",
    );
    assert!(
        fell_through((0b0001, 0b0010)), // disjoint: no pair considered
        "disjoint collision groups still collided",
    );
}

/// Acceptance: "a projectile with matching collision groups but disjoint
/// solver groups detects a wall without being stopped."
///
/// This is the pair of masks earning its keep: one decides whether the pair
/// is looked at, the other whether it pushes.
#[test]
fn matching_collision_groups_with_disjoint_solver_groups_detect_without_stopping() {
    let mut resources = listening_world();
    spawn_body(
        &mut resources,
        Transform::from_position(Vec3::new(0.0, 0.0, 0.0)),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(50.0, 0.5, 50.0),
            // Considered by everything, solved against nothing.
            collision_memberships: u32::MAX,
            collision_filter: u32::MAX,
            solver_memberships: 0b0001,
            solver_filter: 0b0001,
            collision_events: true,
            ..Default::default()
        },
    );
    let projectile = spawn_body(
        &mut resources,
        Transform::from_position(Vec3::new(0.0, 4.0, 0.0)),
        RigidBody::default(),
        Collider {
            collision_memberships: u32::MAX,
            collision_filter: u32::MAX,
            solver_memberships: 0b0010,
            solver_filter: 0b0010,
            collision_events: true,
            ..Default::default()
        },
    );
    Playing::set(&mut resources, true);

    let mut detected = 0;
    for _ in 0..180 {
        frame(&mut resources);
        detected += collected::<CollisionStarted>(&resources).len();
    }

    assert!(detected > 0, "the wall was never detected");
    assert!(
        position(&resources, projectile).y < -2.0,
        "the projectile was stopped at {} despite disjoint solver groups",
        position(&resources, projectile).y,
    );
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Acceptance: "no events survive a Stop." A collision from a session that
/// ended must not be delivered to the next one.
#[test]
fn no_events_survive_a_stop() {
    let mut resources = listening_world();
    spawn_body(
        &mut resources,
        Transform::from_position(Vec3::new(0.0, -1.0, 0.0)),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(50.0, 1.0, 50.0),
            collision_events: true,
            ..Default::default()
        },
    );
    falling_sphere(&mut resources, 2.0);
    Playing::set(&mut resources, true);

    // Land, so there is something in flight to lose.
    let mut seen = 0;
    for _ in 0..120 {
        frame(&mut resources);
        seen += collected::<CollisionStarted>(&resources).len();
    }
    assert!(seen > 0, "setup: nothing collided, nothing to survive");

    // Stop. The lifecycle system runs ungated, so it sees this.
    Playing::set(&mut resources, false);
    frame(&mut resources);

    assert!(
        collected::<CollisionStarted>(&resources).is_empty(),
        "a collision outlived the play session that produced it",
    );
    assert!(collected::<CollisionStopped>(&resources).is_empty());
    assert!(collected::<ContactForce>(&resources).is_empty());
}

/// #560 built joint breaking with nowhere to report it. This is that
/// nowhere filled in.
#[test]
fn a_broken_joint_raises_an_event() {
    let mut resources = listening_world();
    let hook = spawn_body(
        &mut resources,
        Transform::from_position(Vec3::new(0.0, 10.0, 0.0)),
        RigidBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider::default(),
    );
    let load = spawn_body(
        &mut resources,
        Transform::from_position(Vec3::new(0.0, 9.0, 0.0)),
        RigidBody {
            mass: 5.0,
            ..Default::default()
        },
        Collider::default(),
    );
    let joint = spawn_bare(&mut resources);
    insert(
        &mut resources,
        joint,
        Joint {
            kind: JOINT_FIXED,
            body_a: Some(EntityRef::live(hook)),
            body_b: Some(EntityRef::live(load)),
            anchor_a: Vec3::new(0.0, -1.0, 0.0),
            breakable: true,
            break_impulse: 0.02,
            ..Default::default()
        },
    );
    Playing::set(&mut resources, true);

    let breaks = run_collecting::<JointBroke>(&mut resources, 60);
    let event = breaks.first().expect("the joint broke and said nothing");
    assert_eq!(event.joint, joint, "the event names the wrong entity");
    assert_eq!((event.a, event.b), (hook, load));
    assert!(
        event.impulse > 0.02,
        "impulse {} is below the threshold",
        event.impulse
    );
}

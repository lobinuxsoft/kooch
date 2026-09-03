//! Can a project write "you touched the goal"?
//!
//! Only through the **prelude**, deliberately. Every type here was
//! reachable all along at `kooch::kooch_physics::plugin::…`, which is to
//! say: only if you already knew it existed. Reaching past the facade
//! would test the thing that was never broken and miss the one that was
//! (#1065).
//!
//! A goal, a checkpoint and a death plane are one mechanism — a sensor
//! that reports an overlap — so this covers all three.

use crate::prelude::*;

/// The listener a level's goal would be, written the way a project
/// writes one: name the event, read the buffer, react.
fn reached(resources: &Resources, goal: Entity) -> bool {
    resources
        .get::<Events<CollisionStarted>>()
        .is_some_and(|events| {
            events
                .read()
                .any(|hit| hit.sensor && (hit.a == goal || hit.b == goal))
        })
}

/// 🔴 The whole issue: this file compiles.
///
/// `CollisionStarted` could not be named from a project, so a level
/// could not have an end. The character has walked, sprinted, jumped and
/// wall-run since #94 with nothing able to react to any of it.
#[test]
fn a_goal_can_be_written_from_the_prelude() {
    let mut resources = Resources::new();
    resources.insert(Events::<CollisionStarted>::new());
    let goal = Entity::new(7, 0);
    let player = Entity::new(8, 0);

    assert!(!reached(&resources, goal), "reported a touch before any");

    if let Some(events) = resources.get_mut::<Events<CollisionStarted>>() {
        events.send(CollisionStarted {
            a: player,
            b: goal,
            sensor: true,
        });
        // ⚠️ Double-buffered: what is sent this frame is read the next.
        // Deliberate — it stops a listener depending on system order —
        // and the reason a goal fires one frame after the touch.
        events.update();
    }

    assert!(reached(&resources, goal), "the goal never saw the player");
}

/// A solid contact is not a trigger. Standing on the floor must not
/// finish the level.
#[test]
fn a_solid_contact_is_not_a_goal() {
    let mut resources = Resources::new();
    resources.insert(Events::<CollisionStarted>::new());
    let goal = Entity::new(7, 0);

    if let Some(events) = resources.get_mut::<Events<CollisionStarted>>() {
        events.send(CollisionStarted {
            a: Entity::new(8, 0),
            b: goal,
            sensor: false,
        });
        events.update();
    }

    assert!(
        !reached(&resources, goal),
        "a solid contact finished the level",
    );
}

/// The other three the prelude now offers, named so a missing one fails
/// the build here rather than in somebody's game.
#[test]
fn every_physics_event_is_reachable() {
    let a = Entity::new(1, 0);
    let b = Entity::new(2, 0);
    let _ = CollisionStopped {
        a,
        b,
        sensor: false,
    };
    let _ = ContactForce {
        a,
        b,
        total_force_magnitude: 0.0,
        max_force_magnitude: 0.0,
    };
    let _: fn() -> Option<Events<JointBroke>> = || None;
}

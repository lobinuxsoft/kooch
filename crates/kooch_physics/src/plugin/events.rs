//! Turning the solver's reports into engine events about entities.
//!
//! The backend speaks in [`BodyHandle`]; nothing above this seam should
//! have to. These are the types gameplay reads, and they carry [`Entity`].
//!
//! # Why draining is its own system
//!
//! Rapier calls its handler from inside `step`, holding the world mutably.
//! A listener that despawned the thing it collided with would be mutating
//! the set being iterated. So the step collects and this drains afterwards,
//! in [`Stage::PostPhysics`] — by which point the solver has let go.
//!
//! [`Stage::PostPhysics`]: kooch_core::stage::Stage::PostPhysics

use kooch_core::event::Events;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;

use super::world::PhysicsWorld;

/// Two entities started touching.
///
/// Fires once, on the frame contact begins. A listener that wants "is
/// touching right now" should track it from these and
/// [`CollisionStopped`], because the solver does not repeat itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionStarted {
    pub a: Entity,
    pub b: Entity,
    /// Whether this was a sensor overlap rather than a solid contact.
    ///
    /// A sensor has no contact manifold behind it, so a listener wanting a
    /// contact point needs to know not to look.
    pub sensor: bool,
}

/// Two entities stopped touching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionStopped {
    pub a: Entity,
    pub b: Entity,
    pub sensor: bool,
}

/// Two entities hit each other harder than one of them cared to ignore.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactForce {
    pub a: Entity,
    pub b: Entity,
    /// Sum over the contact, in newtons.
    pub total_force_magnitude: f32,
    /// The largest single contact's force — a spread blow and a spike can
    /// share a total.
    pub max_force_magnitude: f32,
}

/// A joint tore off under load.
///
/// #560 built the breaking and had nowhere to report it; this is the
/// nowhere filled in. `joint` is the entity carrying the `Joint` component,
/// which is the one an author recognises.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointBroke {
    pub joint: Entity,
    pub a: Entity,
    pub b: Entity,
    /// The impulse that exceeded the threshold.
    pub impulse: f32,
}

/// Drains the solver's reports into the engine's event buffers.
///
/// Runs after the step, for the reason in the module docs. Registered in
/// [`Stage::PostPhysics`](kooch_core::stage::Stage::PostPhysics) alongside
/// writeback, and gated on play like the rest of gameplay.
pub(super) fn drain_physics_events(resources: &mut Resources) {
    let Some(mut world) = resources.remove::<PhysicsWorld>() else {
        return;
    };

    let collisions = world.backend_mut().take_collision_events();
    let forces = world.backend_mut().take_contact_force_events();

    // Resolved while the world is in hand: slot → entity needs it, and the
    // event buffers do not.
    let collisions: Vec<(Entity, Entity, bool, bool)> = collisions
        .into_iter()
        .filter_map(|event| {
            Some((
                world.entity_of(event.a)?,
                world.entity_of(event.b)?,
                event.started,
                event.sensor,
            ))
        })
        .collect();
    let forces: Vec<ContactForce> = forces
        .into_iter()
        .filter_map(|event| {
            Some(ContactForce {
                a: world.entity_of(event.a)?,
                b: world.entity_of(event.b)?,
                total_force_magnitude: event.total_force_magnitude,
                max_force_magnitude: event.max_force_magnitude,
            })
        })
        .collect();
    let breaks = std::mem::take(world.joints_mut().drained_breaks());

    resources.insert(world);

    for (a, b, started, sensor) in collisions {
        report(a, b, started, sensor);
        match started {
            true => send(resources, CollisionStarted { a, b, sensor }),
            false => send(resources, CollisionStopped { a, b, sensor }),
        }
    }
    for event in forces {
        send(resources, event);
    }
    for event in breaks {
        send(resources, event);
    }
}

/// Says a collision happened, so it is visible without writing a listener.
///
/// # Why a sensor is louder than a contact
///
/// A trigger firing is a gameplay event: something is meant to react, and
/// "did my trigger fire" is a question with no other way to answer it — the
/// body passes through and nothing moves. Solid contacts are constant by
/// comparison; a scene at rest still generates them, and a stack of crates
/// would bury everything else.
///
/// So sensors are `info` and contacts are `debug`. Both are gated by
/// `RUST_LOG` like anything else, and neither is a substitute for a
/// listener — this exists so that a scene can be understood before anyone
/// writes one.
fn report(a: Entity, b: Entity, started: bool, sensor: bool) {
    match (sensor, started) {
        (true, true) => tracing::info!(
            target: "kooch_physics",
            a = a.index(),
            b = b.index(),
            "a sensor was entered",
        ),
        (true, false) => tracing::info!(
            target: "kooch_physics",
            a = a.index(),
            b = b.index(),
            "a sensor was left",
        ),
        (false, true) => tracing::debug!(
            target: "kooch_physics",
            a = a.index(),
            b = b.index(),
            "two bodies started touching",
        ),
        (false, false) => tracing::debug!(
            target: "kooch_physics",
            a = a.index(),
            b = b.index(),
            "two bodies stopped touching",
        ),
    }
}

/// Sends an event if the app registered its buffer.
///
/// A host that never called `add_event` gets silence rather than a panic:
/// physics is usable without anyone listening.
fn send<E: Send + Sync + 'static>(resources: &mut Resources, event: E) {
    if let Some(events) = resources.get_mut::<Events<E>>() {
        events.send(event);
    }
}

/// Whether physics saw the app playing last frame.
///
/// There is no stop *event* to listen for — [`Playing`] is a flag someone
/// flips — so the transition has to be noticed by remembering. Kept inside
/// the physics plugin because nothing else needs to care.
///
/// [`Playing`]: kooch_core::run_state::Playing
#[derive(Debug, Default)]
pub(super) struct WasPlaying(pub(super) bool);

/// Clears the event buffers on the frame play stops.
///
/// Runs unconditionally, unlike the drain: a system gated on play cannot
/// see play end.
pub(super) fn physics_lifecycle_system(resources: &mut Resources) {
    let playing = kooch_core::run_state::Playing::is_playing(resources);
    let was = resources
        .get::<WasPlaying>()
        .map(|state| state.0)
        .unwrap_or(false);
    if was == playing {
        return;
    }
    match resources.get_mut::<WasPlaying>() {
        Some(state) => state.0 = playing,
        None => {
            resources.insert(WasPlaying(playing));
        }
    }
    // On stop, and on start too: a session beginning should not inherit
    // whatever the editor's authoring-time world happened to report.
    clear_physics_events(resources);
}

/// Clears every physics event buffer.
///
/// A collision from a play session that has ended must not be delivered to
/// the next one. Both halves matter: the backend's queues are drained *and*
/// the engine's buffers cleared, because an event already translated is
/// still an event about a world that no longer exists.
pub(super) fn clear_physics_events(resources: &mut Resources) {
    if let Some(mut world) = resources.remove::<PhysicsWorld>() {
        let _ = world.backend_mut().take_collision_events();
        let _ = world.backend_mut().take_contact_force_events();
        world.joints_mut().drained_breaks().clear();
        resources.insert(world);
    }
    clear::<CollisionStarted>(resources);
    clear::<CollisionStopped>(resources);
    clear::<ContactForce>(resources);
    clear::<JointBroke>(resources);
}

fn clear<E: Send + Sync + 'static>(resources: &mut Resources) {
    if let Some(events) = resources.get_mut::<Events<E>>() {
        // Two swaps: `update` moves write into read, so one call would
        // leave the events that were pending readable for another frame.
        events.update();
        events.update();
    }
}

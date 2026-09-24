//! Solver reports as entity events for gameplay. Drained in
//! [`Stage::PostPhysics`](kooch_core::stage::Stage::PostPhysics), after `step` releases the world a
//! listener might mutate.

use kooch_core::event::Events;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;

use super::world::PhysicsWorld;

/// Two entities started touching, once; track "touching now" with [`CollisionStopped`], since the
/// solver does not repeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionStarted {
    pub a: Entity,
    pub b: Entity,
    /// A sensor overlap, with no contact manifold behind it.
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

/// A joint tore off under load (#560); `joint` is the entity carrying the `Joint`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointBroke {
    pub joint: Entity,
    pub a: Entity,
    pub b: Entity,
    /// The impulse that exceeded the threshold.
    pub impulse: f32,
}

/// Drains solver reports into event buffers in
/// [`Stage::PostPhysics`](kooch_core::stage::Stage::PostPhysics), gated on play.
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

/// Logs a collision so a scene is understandable before any listener: sensors at `info` (a trigger
/// is gameplay, and passes through silently), contacts at `debug` (a resting stack floods).
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

/// Sends if the app registered the buffer; without `add_event`, silence, not a panic.
fn send<E: Clone + Send + Sync + 'static>(resources: &mut Resources, event: E) {
    if let Some(events) = resources.get_mut::<Events<E>>() {
        events.send(event);
    }
}

/// Whether physics saw play last frame: [`Playing`](kooch_core::run_state::Playing) is a flag, so
/// stopping is noticed by remembering.
#[derive(Debug, Default)]
pub(super) struct WasPlaying(pub(super) bool);

/// Clears event buffers when play stops; ungated, since a play-gated system cannot see play end.
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

/// Clears backend queues and engine buffers, so a finished session's collisions never reach the
/// next.
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

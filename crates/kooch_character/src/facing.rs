//! [`Facing`] — which way a character is being steered.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Where gameplay is steering a character, in world space.
///
/// Written by gameplay **every frame**, read by
/// [`CharacterPlugin`](crate::CharacterPlugin), which turns the body
/// towards it at
/// [`turn_speed`](crate::CharacterController::turn_speed) and walks at
/// [`Walk::max_speed`](crate::Walk::max_speed) times its length.
///
/// # Why this is not read off the velocity
///
/// A floating capsule's velocity carries the spring's own oscillation
/// and every slope it is being pushed along, so a character reading it
/// twitches while standing still on a ramp. Steering is intent, and
/// intent is an input.
///
/// # Zero is a released stick, not a missing one
///
/// The length is the throttle, so zero has to mean **stop**, and a
/// system that skips writing it leaves the throttle wherever it was —
/// a character walking on its own for ever, which is exactly what
/// happened.
///
/// The heading is kept without keeping the throttle: with nothing to
/// steer by, the turn falls back to the way the body is already
/// looking, so releasing the stick stops the character where it stands
/// rather than snapping it to face whatever direction zero names.
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect)]
#[reflect(category = "Physics")]
pub struct Facing {
    /// Direction and throttle in one: flattened against the local up,
    /// and its length clamped to `1` before it scales the top speed.
    pub direction: Vec3,
}

impl Component for Facing {}

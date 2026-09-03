//! [`Facing`] — which way a character is being steered.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// The direction a character should turn to face, in world space.
///
/// Written by gameplay, read by
/// [`CharacterPlugin`](crate::CharacterPlugin), which turns the body
/// towards it about the local up at
/// [`turn_speed`](crate::CharacterController::turn_speed).
///
/// # Why this is not read off the velocity
///
/// A floating capsule's velocity carries the spring's own oscillation
/// and every slope it is being pushed along, so a character reading it
/// twitches while standing still on a ramp. Steering is intent, and
/// intent is an input.
///
/// # Zero means keep looking
///
/// A character that let go of the stick would otherwise snap back to
/// whatever direction zero happens to name. The last direction stands
/// until a new one is written.
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect)]
#[reflect(category = "Physics")]
pub struct Facing {
    /// Any length; only the direction is read, flattened against the
    /// local up.
    pub direction: Vec3,
}

impl Component for Facing {}

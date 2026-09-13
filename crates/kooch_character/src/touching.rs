//! [`Touching`] — the wall a character is up against.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// What the character is pressed against, ahead — the horizontal twin of
/// [`Grounded`](crate::Grounded), written by the same pass. Too steep to walk and within reach, not
/// stuck to it.
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect)]
#[reflect(category = "Physics")]
pub struct Touching {
    /// Something too steep to stand on was found within
    /// [`reach`](crate::CharacterController::reach).
    pub wall: bool,
    /// Its world-space surface normal, or [`Vec3::ZERO`] for nothing.
    ///
    /// What a wall jump pushes off and what a slide runs along.
    pub normal: Vec3,
    /// How far ahead it is.
    pub distance: f32,
}

impl Component for Touching {}

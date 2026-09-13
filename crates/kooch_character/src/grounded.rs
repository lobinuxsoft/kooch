//! [`Grounded`] — the one answer to "am I standing on something".

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// What the controller found under a character this step, written once for jumping, animation,
/// audio and fall damage so they cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect)]
#[reflect(category = "Physics")]
pub struct Grounded {
    /// Ground was found, no steeper than [`max_slope`](crate::CharacterController::max_slope). A
    /// wall is ground with this still false.
    pub standing: bool,
    /// World-space surface normal under the character, or [`Vec3::ZERO`] when nothing was found —
    /// the slope to lean and blend towards.
    pub normal: Vec3,
    /// Gap between the capsule and that surface — not zero while standing, since the spring holds
    /// it open.
    pub distance: f32,
}

impl Component for Grounded {}

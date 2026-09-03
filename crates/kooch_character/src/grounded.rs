//! [`Grounded`] — the one answer to "am I standing on something".

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// What the controller found under a character this step.
///
/// # Why this is a component and not a raycast in each caller
///
/// Jumping, animation, footstep audio and fall damage all ask the same
/// question, and each answering it alone means four sweeps a frame for
/// one fact — and four chances to use a slightly different probe length
/// and disagree about whether the character is on the floor.
///
/// Written every step by [`CharacterPlugin`](crate::CharacterPlugin), so
/// a reader never has to know how it was found.
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect)]
#[reflect(category = "Physics")]
pub struct Grounded {
    /// Ground was found, and it is not steeper than
    /// [`max_slope`](crate::CharacterController::max_slope).
    ///
    /// A wall is ground the controller cannot stand on: something is
    /// under the probe, and `standing` is still false.
    pub standing: bool,
    /// World-space surface normal under the character, or
    /// [`Vec3::ZERO`] when nothing was found.
    ///
    /// The slope, which is what a controller leans into and what an
    /// animation blends towards.
    pub normal: Vec3,
    /// Gap between the capsule and that surface.
    ///
    /// Not zero while standing: the capsule floats, and this is the
    /// distance the spring is holding open.
    pub distance: f32,
}

impl Component for Grounded {}

//! [`Walk`] — how a character gets up to speed, and back down again.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Walking, as a velocity the controller is asked to reach.
///
/// Attach beside a [`CharacterController`](crate::CharacterController).
/// The direction comes from [`Facing`](crate::Facing), whose length is
/// the throttle.
///
/// # Why a goal velocity and not a push
///
/// A floating capsule never touches the floor, so it has **no friction
/// at all**. A controller that pushes in the input direction and stops
/// pushing at top speed has nothing left to slow the body down: let go
/// and it coasts for ever.
///
/// Asking for a velocity instead makes stopping the same mechanism as
/// starting — release the stick and the goal is zero, so the same force
/// that got the character moving is what brings it back. Friction, the
/// top speed and a hard stop all fall out of one term rather than being
/// three that have to agree.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct Walk {
    /// Top speed across the ground, in m/s. The goal never exceeds it,
    /// so nothing has to clamp afterwards.
    pub max_speed: f32,
    /// How fast the goal itself may change, in m/s².
    ///
    /// The feel of the controls: this is how long the character takes to
    /// agree with the stick, before any force is worked out.
    pub acceleration: f32,
    /// Ceiling on the acceleration used to reach the goal, in m/s².
    ///
    /// Separate from `acceleration` because they answer different
    /// questions. The goal can change instantly and still be chased
    /// gently, which is what lets a character be responsive without
    /// being able to shove a heavy crate across the room.
    pub max_force: f32,
    /// How much of that still applies with nothing under the body, as a
    /// fraction.
    ///
    /// `0` is a jump you cannot steer; `1` is thrust, and on a planet
    /// thrust climbs out of its own orbit.
    pub air_control: f32,
    /// How far the body tilts into its own acceleration, as a fraction
    /// of the tilt that would balance it.
    ///
    /// `0` stands straight up. `1` leans as far as the acceleration
    /// could hold, which is what a body braking hard actually does. It
    /// is drawn into the turn rather than applied as an off-centre push,
    /// because the orientation is authored — see
    /// [`turn_speed`](crate::CharacterController::turn_speed).
    pub lean: f32,
}

impl Default for Walk {
    fn default() -> Self {
        Self {
            max_speed: 6.0,
            acceleration: 60.0,
            max_force: 90.0,
            air_control: 0.3,
            lean: 0.35,
        }
    }
}

impl Component for Walk {}

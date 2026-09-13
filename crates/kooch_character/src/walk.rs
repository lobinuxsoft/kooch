//! [`Walk`] — how a character gets up to speed, and back down again.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Walking as a velocity the controller is asked to reach, steered by [`Facing`](crate::Facing).
/// A goal velocity, not a push: a floating capsule has no friction, so stopping has to be the same
/// term as starting.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct Walk {
    /// Top speed across the ground, in m/s. The goal never exceeds it,
    /// so nothing has to clamp afterwards.
    pub max_speed: f32,
    /// How fast the goal itself may change, in m/s² — how long the character takes to agree with
    /// the stick.
    pub acceleration: f32,
    /// Ceiling on the acceleration used to reach the goal, in m/s² — responsive controls without
    /// shoving heavy crates.
    pub max_force: f32,
    /// How much of that applies in the air: `0` is an unsteerable jump, `1` is thrust that climbs
    /// out of orbit.
    pub air_control: f32,
    /// How far the body tilts into its acceleration, as a fraction of the balancing tilt — drawn
    /// into the turn since the orientation is authored.
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

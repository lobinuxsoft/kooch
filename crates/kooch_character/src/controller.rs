//! [`CharacterController`] — the numbers the floating capsule is tuned by.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// A body held above the ground by a spring instead of resting on it — beside a dynamic
/// `PhysicsBody` and a `Collider`, which is its shape. Tuned for a two-metre capsule floating a
/// quarter metre.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct CharacterController {
    /// How high the body's **origin** rides above the ground, in metres.
    /// It must exceed the collider's reach below the origin (`r + h` for a capsule), or the capsule
    /// rests on the floor; the gap is also the step height.
    pub ride_height: f32,
    /// How far below the body to look for ground, in metres; past `ride_height`, how far it can
    /// drop before it counts as falling.
    pub probe: f32,
    /// Radius of the probing sphere, near the character's own — much smaller finds the gap between
    /// floor tiles.
    pub probe_radius: f32,
    /// How hard the spring pulls the body back to `ride_height`, as an
    /// acceleration per metre of error.
    pub stiffness: f32,
    /// How strongly the spring resists vertical speed — the landing dial. Critical is
    /// `2·sqrt(stiffness)`; about a third of it dips once and recovers, which reads as weight.
    pub damping: f32,
    /// How quickly the body turns to stand on the local up and face its steering, in turns per
    /// second. Set, not torqued: without the inertia tensor an angular spring could only be tuned
    /// by feel.
    pub turn_speed: f32,
    /// Steepest ground that still counts as standing, in degrees. Steeper is a wall:
    /// [`Grounded`](crate::Grounded) reports it, but the spring does not hold the body up against
    /// it.
    pub max_slope: f32,
    /// The tallest obstruction that counts as a step, in metres. A riser and a wall share a normal,
    /// so a step is one with a ledge found this far above the contact.
    pub step_height: f32,
    /// How far ahead to look for a wall, in metres, written to [`Touching`](crate::Touching) — a
    /// little more than the character's radius.
    pub reach: f32,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            // A capsule of radius 0.4 and half-height 0.5 reaches 0.9
            // below its origin, so this floats it by 0.2.
            ride_height: 1.1,
            probe: 1.8,
            probe_radius: 0.35,
            stiffness: 90.0,
            // A third of critical (`2·sqrt(90)` is 19), so a landing
            // dips and recovers instead of arriving dead.
            damping: 7.0,
            turn_speed: 10.0,
            max_slope: 50.0,
            step_height: 0.5,
            reach: 0.7,
        }
    }
}

impl Component for CharacterController {}

impl CharacterController {
    /// Whether a surface with this normal can be stood on, given which
    /// way is up here.
    pub fn stands_on(&self, normal: glam::Vec3, up: glam::Vec3) -> bool {
        let Some(normal) = normal.try_normalize() else {
            return false;
        };
        normal.dot(up) >= self.max_slope.to_radians().cos()
    }
}

#[cfg(test)]
mod tests;

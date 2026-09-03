//! [`CharacterController`] — the numbers the floating capsule is tuned by.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// A body held above the ground by a spring instead of resting on it.
///
/// Attach beside a `PhysicsBody` (dynamic) and a `Collider`. The collider
/// is the character's shape; this only describes how it is held up.
///
/// # Default
///
/// Tuned for a capsule about two metres tall floating a quarter of a
/// metre, which is a person who can walk up a kerb without noticing.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct CharacterController {
    /// How high the body's **origin** rides above the ground, in metres.
    ///
    /// The one number that decides how a character feels, and the one
    /// nothing else in the editor draws.
    ///
    /// # It must clear the collider
    ///
    /// Measured from the origin, not from the capsule's feet, because
    /// the controller does not know where its feet are — the collider
    /// could be any shape, offset any way. So this has to exceed the
    /// collider's own reach below the origin, or the spring is asking
    /// for a height the geometry cannot occupy and the capsule simply
    /// rests on the floor instead of floating.
    ///
    /// For a capsule of radius `r` and half-height `h`, that reach is
    /// `r + h`. The gizmo draws both, so a value that cannot be reached
    /// is visible rather than something to work out.
    ///
    /// The clearance it buys is also the step height: a step shorter
    /// than the gap is climbed by the spring alone, with no code that
    /// knows what a step is.
    pub ride_height: f32,
    /// How far below the body to look for ground, in metres.
    ///
    /// Must exceed `ride_height`, or the probe ends before the rest
    /// position and the character can never find the floor it is
    /// standing on. Past `ride_height` the extra is how far it can drop
    /// before it counts as falling.
    pub probe: f32,
    /// Radius of the sphere that does the looking.
    ///
    /// Wants to be near the character's own radius. A sphere much
    /// smaller behaves like the ray this is not, and finds the gap
    /// between two floor tiles.
    pub probe_radius: f32,
    /// How hard the spring pulls the body back to `ride_height`, as an
    /// acceleration per metre of error.
    pub stiffness: f32,
    /// How strongly the spring resists vertical speed.
    ///
    /// This is the feel dial. Critical damping is `2·sqrt(stiffness)`;
    /// the fraction of it you choose is what a landing looks like. At
    /// the full value the character arrives dead, with no dip and no
    /// recovery — correct, and it reads as a body with no weight. Around
    /// a third of it dips once and comes back up, which is the landing
    /// people mean when they say a character has weight.
    ///
    /// Too little and it never settles; too much and it sinks into a
    /// step instead of rising over it.
    pub damping: f32,
    /// How quickly the body turns to stand on the local up and face
    /// where it is steered, in turns per second towards the target.
    ///
    /// The orientation is set rather than torqued. Correcting a rotation
    /// with a torque needs the inertia tensor, which the backend does
    /// not expose, so an angular spring here could only ever be tuned by
    /// feel — and it was, badly: a value that settled on flat ground
    /// wallowed on a planet. A character's orientation is authored, not
    /// simulated.
    ///
    /// The cost is that it no longer spins when something hits it. For a
    /// character that is the point.
    pub turn_speed: f32,
    /// Steepest ground that still counts as standing, in degrees.
    ///
    /// Above it the surface is a wall. The sweep still finds it and
    /// [`Grounded`] still reports its normal, so an animation can see
    /// it — but the spring does not hold the body up against it. It
    /// would have to cancel gravity to do so, and a slope the controller
    /// has already refused to walk would carry the character to the top
    /// of it.
    ///
    /// [`Grounded`]: crate::Grounded
    pub max_slope: f32,
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

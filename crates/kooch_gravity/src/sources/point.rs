//! [`PointGravity`] — a field pulling towards one point.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// A field pulling towards this entity — a planet, a moon, a black hole — moving and parenting with
/// it.
///
/// Full `strength` everywhere inside `radius`, fading to nothing across `falloff` past it: the same
/// shape of reach as every other bounded source, so a [`GravityPriority`](crate::GravityPriority)
/// takes over a planet's surroundings the way it takes over a room.
///
/// # Sizing one
///
/// A body stays grounded while gravity covers its centripetal acceleration, `r` being the body's
/// centre — the planet's surface plus the body's own radius:
///
/// ```text
/// stays on the ground   v <= sqrt(g · r)
/// leaves for good       v >= sqrt( g · (2·(radius − r) + falloff) )
/// ```
///
/// Twice the speed needs four times the planet — at 9.81, 8 m/s wants a centre 6.5 m out, and
/// 20 m/s wants 41 m. More `strength` costs jump height, `(J/m)²/(2·g)`: 9.81 → 25 buys 2.5× the
/// ground speed and costs 60 % of the jump.
///
/// Past `radius + falloff`, [`gravity_up`](crate::gravity_up) answers world up and the controls
/// turn world-relative — keep the reach past anything the player reaches.
///
/// Metres, like every source: scaling the entity moves the field without resizing it. Defaults to
/// Earth's pull out to 50 m.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct PointGravity {
    /// Acceleration anywhere inside [`radius`](Self::radius), in m/s².
    pub strength: f32,
    /// How far the field reaches at full strength, in metres. Zero or less is unlimited.
    ///
    /// 🔴 Aliased from `range`: that was the reach before a separate reference radius was dropped,
    /// and a saved scene writes `radius` before `range`, so the reach is what an old planet keeps.
    #[reflect(alias = "range")]
    pub radius: f32,
    /// How far past `radius` the field fades to nothing, in metres — the pull and, with a
    /// [`GravityPriority`](crate::GravityPriority), the claim over lower levels. Zero is a hard
    /// edge, which a priority turns into a jolt: the planet takes over all at once.
    pub falloff: f32,
}

impl Default for PointGravity {
    fn default() -> Self {
        Self {
            strength: 9.81,
            radius: 50.0,
            falloff: 10.0,
        }
    }
}

impl Component for PointGravity {}

impl PointGravity {
    /// The acceleration this source applies at `point`, given its own
    /// world position.
    pub fn acceleration_at(&self, source: Vec3, point: Vec3) -> Vec3 {
        let offset = source - point;
        // At the centre there is no direction to pull in, and dividing by
        // it would produce NaN that outlives the frame.
        let Some(direction) = offset.try_normalize() else {
            return Vec3::ZERO;
        };
        direction * self.strength * self.influence(offset.length())
    }

    /// How strongly the field reaches `distance` metres out: 1 inside `radius`, fading to 0 across
    /// `falloff`, 0 beyond. Unlimited when `radius` is zero or less.
    pub fn influence(&self, distance: f32) -> f32 {
        if self.radius <= 0.0 || distance <= self.radius {
            return 1.0;
        }
        if self.falloff <= 0.0 {
            return 0.0;
        }
        (1.0 - (distance - self.radius) / self.falloff).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests;

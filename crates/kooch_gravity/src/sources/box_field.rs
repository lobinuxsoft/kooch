//! [`BoxGravity`] — a cube planet, pulling towards its nearest surface.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// A solid box pulling towards the nearest point of its surface — its SDF gradient — so faces are
/// walkable and edges turn with no special case.
/// Acts outside, unlike [`super::AreaGravity`]; metres, unscaled.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct BoxGravity {
    /// Half-extents of the solid, in metres.
    pub half_extents: Vec3,
    /// Acceleration at the surface, in metres per second squared.
    pub strength: f32,
    /// How gently gravity turns around the edges, in metres: the box shrinks by this before the
    /// nearest point is taken. Zero is a cube; the half-extents make a sphere.
    pub rounding: f32,
    /// How far from the surface the field holds at full strength, in metres; zero or less is
    /// unlimited and disables `falloff`.
    pub range: f32,
    /// How far past `range` the field fades to nothing, in metres, so leaving is not an instant
    /// loss. Zero for a hard cutoff.
    pub falloff: f32,
}

impl Default for BoxGravity {
    fn default() -> Self {
        Self {
            half_extents: Vec3::splat(5.0),
            strength: 9.81,
            rounding: 0.5,
            range: 20.0,
            falloff: 5.0,
        }
    }
}

impl Component for BoxGravity {}

impl BoxGravity {
    /// The acceleration this source applies at a point already expressed
    /// in the source's local space.
    pub fn acceleration_at_local(&self, local_point: Vec3) -> Vec3 {
        let Some((direction, distance)) = self.pull_at_local(local_point) else {
            return Vec3::ZERO;
        };
        direction * self.strength * self.influence(distance)
    }

    /// The direction towards the surface and the distance to it, or `None` inside — the direction
    /// alone is what a controller asks.
    pub fn pull_at_local(&self, local_point: Vec3) -> Option<(Vec3, f32)> {
        // Shrinking by `rounding` and measuring from the shrunk box is the
        // whole of the rounded-box distance function. Clamped at zero so an
        // over-large rounding gives a sphere rather than an inside-out box.
        let half = (self.half_extents.abs() - Vec3::splat(self.rounding.max(0.0))).max(Vec3::ZERO);
        let offset = local_point.clamp(-half, half) - local_point;

        let reach = offset.length();
        // Inside the solid there is no surface to fall towards, and at the
        // exact centre no direction either. Both are the same answer.
        if reach <= self.rounding.max(0.0) {
            return None;
        }
        let direction = offset.try_normalize()?;
        Some((direction, reach - self.rounding.max(0.0)))
    }

    /// How strongly the field applies at a distance from the surface: 1 up
    /// to `range`, fading to 0 across `falloff`.
    pub fn influence(&self, distance: f32) -> f32 {
        if self.range <= 0.0 || distance <= self.range {
            return 1.0;
        }
        if self.falloff <= 0.0 {
            return 0.0;
        }
        (1.0 - (distance - self.range) / self.falloff).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests;

//! [`PlaneGravity`] — one-sided uniform pull towards an infinite plane.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// A floor of unbounded extent pulling along its local `normal` — one-sided and height-limited,
/// unlike [`super::GlobalGravity`].
/// Metres, unscaled; defaults to Earth-strength over 50 m, fading across 10.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct PlaneGravity {
    /// Which way is up out of the plane, in local space — the pull is its negation, stated as the
    /// surface's own normal.
    pub normal: Vec3,
    /// Acceleration in metres per second squared.
    pub strength: f32,
    /// How far above the plane the field holds at full strength, in metres; zero or less is
    /// unlimited and disables `falloff`.
    pub range: f32,
    /// How far past `range` the field fades to nothing, in metres, so leaving is not an instant
    /// loss. Zero for a hard cutoff.
    pub falloff: f32,
}

impl Default for PlaneGravity {
    fn default() -> Self {
        Self {
            normal: Vec3::Y,
            strength: 9.81,
            range: 50.0,
            falloff: 10.0,
        }
    }
}

impl Component for PlaneGravity {}

impl PlaneGravity {
    /// The acceleration this source applies at a point already expressed in
    /// the source's local space.
    pub fn acceleration_at_local(&self, local_point: Vec3) -> Vec3 {
        let Some(normal) = self.normal.try_normalize() else {
            return Vec3::ZERO;
        };
        -normal * self.strength * self.influence_at_local(local_point)
    }

    /// How strongly the field applies at a local point: 1 up to `range`, fading across `falloff`, 0
    /// below — a two-sided plane would trap a body.
    pub fn influence_at_local(&self, local_point: Vec3) -> f32 {
        let Some(normal) = self.normal.try_normalize() else {
            return 0.0;
        };
        let height = local_point.dot(normal);
        if height < 0.0 {
            return 0.0;
        }
        if self.range <= 0.0 || height <= self.range {
            return 1.0;
        }
        if self.falloff <= 0.0 {
            return 0.0;
        }
        (1.0 - (height - self.range) / self.falloff).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests;

//! [`AreaGravity`] — a region with its own uniform down.

use glam::Vec3;

use ome_ecs::Reflect;
use ome_ecs::component::Component;

/// A field with its own direction, inside a box.
///
/// The level with its own down: a corridor that runs up a wall, a room
/// that flips over. The box is centred on the entity and rotates with it,
/// so `direction` is given in the entity's local space and turning the
/// entity turns the field.
///
/// This is a region you are *inside*. For a solid you stand on the outside
/// of, whose direction changes around it, see [`super::BoxGravity`].
///
/// # Default
///
/// Earth-strength, downward, in a 10 m cube.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct AreaGravity {
    /// Which way is down, in the entity's local space.
    pub direction: Vec3,
    /// Acceleration in metres per second squared.
    pub strength: f32,
    /// Half-extents of the affected box, in the entity's local space.
    pub half_extents: Vec3,
    /// How far outside the box the field fades to nothing.
    ///
    /// Without it a body crossing the boundary changes direction between
    /// one step and the next, which reads as a jolt. Zero for a hard edge.
    pub falloff: f32,
}

impl Default for AreaGravity {
    fn default() -> Self {
        Self {
            direction: Vec3::NEG_Y,
            strength: 9.81,
            half_extents: Vec3::splat(5.0),
            falloff: 1.0,
        }
    }
}

impl Component for AreaGravity {}

impl AreaGravity {
    /// The acceleration this source applies at a point already expressed
    /// in the source's local space.
    ///
    /// Local space because the box rotates with the entity: converting the
    /// point once is cheaper and clearer than rotating the box.
    pub fn acceleration_at_local(&self, local_point: Vec3) -> Vec3 {
        let Some(direction) = self.direction.try_normalize() else {
            return Vec3::ZERO;
        };
        direction * self.strength * self.influence_at_local(local_point)
    }

    /// How strongly the field applies at a local point: 1 inside the box,
    /// falling to 0 across `falloff`, and 0 beyond.
    pub fn influence_at_local(&self, local_point: Vec3) -> f32 {
        let half = self.half_extents.abs();
        // Distance outside the box, per axis. Inside, every component is
        // zero and the length is zero.
        let outside = (local_point.abs() - half).max(Vec3::ZERO);
        let distance = outside.length();
        if distance <= 0.0 {
            return 1.0;
        }
        if self.falloff <= 0.0 {
            return 0.0;
        }
        (1.0 - distance / self.falloff).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_area_is_at_full_strength_inside_its_box() {
        let area = AreaGravity::default();
        assert_eq!(area.influence_at_local(Vec3::ZERO), 1.0);
        assert_eq!(area.influence_at_local(Vec3::new(4.9, 0.0, 0.0)), 1.0);
    }

    /// A body crossing the boundary should not change direction between
    /// one step and the next.
    #[test]
    fn an_area_fades_across_its_falloff() {
        let area = AreaGravity {
            falloff: 2.0,
            ..Default::default()
        };
        let half_way = area.influence_at_local(Vec3::new(6.0, 0.0, 0.0));
        assert!(
            (half_way - 0.5).abs() < 1e-3,
            "one metre past a 5 m box with 2 m falloff should be half: {half_way}",
        );
        assert_eq!(area.influence_at_local(Vec3::new(7.1, 0.0, 0.0)), 0.0);
    }

    #[test]
    fn a_hard_edged_area_stops_at_its_boundary() {
        let area = AreaGravity {
            falloff: 0.0,
            ..Default::default()
        };
        assert_eq!(area.influence_at_local(Vec3::new(5.1, 0.0, 0.0)), 0.0);
        assert_eq!(area.influence_at_local(Vec3::new(4.9, 0.0, 0.0)), 1.0);
    }

    /// A direction mid-edit passes through zero, and a normalise of zero
    /// is a NaN that outlives the typo.
    #[test]
    fn a_degenerate_direction_applies_nothing() {
        let area = AreaGravity {
            direction: Vec3::ZERO,
            ..Default::default()
        };
        assert_eq!(area.acceleration_at_local(Vec3::ZERO), Vec3::ZERO);
    }
}

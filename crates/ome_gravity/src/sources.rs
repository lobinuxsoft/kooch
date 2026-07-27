//! The three shapes a gravity field comes in.
//!
//! # Three components, not one with a discriminant
//!
//! Physics spells its variants as a reflected `u32` — `Collider.shape`,
//! `Joint.kind` — because reflection has no enum representation and a
//! collider is *one* shape at a time. Gravity is different in the way that
//! matters: a scene has many sources at once, they are queried
//! independently, and an entity is never "a point source that is also an
//! area". Separate components let the archetype answer the query instead
//! of a filter over a discriminant, and make an invalid combination
//! unrepresentable rather than merely unlikely.
//!
//! # Fields add
//!
//! Overlapping sources sum, because that is what gravity does. Two planets
//! near each other pull along the vector sum, and the transition between
//! them is smooth without anyone choosing a blending weight — superposition
//! is the blend.
//!
//! What summing does not express is a zone that *replaces*: "inside this
//! room, down is -X, ignore the planet". That wants a priority rather than
//! a weight, and is deliberately absent until something asks for it.

use glam::Vec3;

use ome_ecs::Reflect;
use ome_ecs::component::Component;

/// A uniform field with no source and no falloff.
///
/// What every scene has by default, expressed as a component so it can be
/// authored, moved between scenes, and switched off — rather than living
/// only in the plugin's configuration where a level cannot reach it.
///
/// # Default
///
/// Earth, downward.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct GlobalGravity {
    /// Acceleration in metres per second squared, in world space.
    pub acceleration: Vec3,
}

impl Default for GlobalGravity {
    fn default() -> Self {
        Self {
            acceleration: Vec3::new(0.0, -9.81, 0.0),
        }
    }
}

impl Component for GlobalGravity {}

/// A field pulling towards this entity — a planet, a moon, a black hole.
///
/// The direction is towards the entity's own position, so moving the
/// entity moves the field, and parenting it to something makes the field
/// follow.
///
/// # Default
///
/// Roughly Earth's surface gravity at a 50 m radius, which is a planet you
/// can walk around in a test scene rather than one you would need a
/// telescope to see.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct PointGravity {
    /// Acceleration at [`radius`](Self::radius), in metres per second
    /// squared.
    ///
    /// Given at a distance rather than as a mass so it can be authored
    /// directly: "9.81 at the surface" is a number someone can reason
    /// about, and `G·M` is not.
    pub strength: f32,
    /// The distance at which the field is exactly `strength`.
    pub radius: f32,
    /// Beyond this, the source contributes nothing.
    ///
    /// A cutoff rather than an infinite field: real gravity never reaches
    /// zero, and summing every source in a galaxy for every body is a cost
    /// with no gameplay behind it. Zero or less means unlimited.
    pub range: f32,
    /// Fall off with the square of distance, as gravity does.
    ///
    /// Off gives a field of constant strength inside `range`, which is not
    /// physical and is often what a game wants: a small planet you can
    /// walk on without the pull changing under your feet.
    pub inverse_square: bool,
}

impl Default for PointGravity {
    fn default() -> Self {
        Self {
            strength: 9.81,
            radius: 50.0,
            range: 500.0,
            inverse_square: true,
        }
    }
}

impl Component for PointGravity {}

/// A field with its own direction, inside a box.
///
/// The level with its own down: a corridor that runs up a wall, a room
/// that flips over. The box is centred on the entity and rotates with it,
/// so `direction` is given in the entity's local space and turning the
/// entity turns the field.
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

impl PointGravity {
    /// The acceleration this source applies at `point`, given its own
    /// world position.
    pub fn acceleration_at(&self, source: Vec3, point: Vec3) -> Vec3 {
        let offset = source - point;
        let distance = offset.length();
        // At the centre there is no direction to pull in, and dividing by
        // it would produce NaN that outlives the frame.
        let Some(direction) = offset.try_normalize() else {
            return Vec3::ZERO;
        };
        if self.range > 0.0 && distance > self.range {
            return Vec3::ZERO;
        }

        let magnitude = match self.inverse_square {
            // Clamped at the reference radius rather than growing without
            // bound: inside a planet the pull should not go to infinity as
            // a body approaches the centre, which is both unphysical and a
            // reliable way to launch something out of the world.
            true => self.strength * (self.radius / distance.max(self.radius)).powi(2),
            false => self.strength,
        };
        direction * magnitude
    }
}

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
    fn a_point_source_pulls_towards_itself() {
        let source = PointGravity::default();
        let accel = source.acceleration_at(Vec3::ZERO, Vec3::new(50.0, 0.0, 0.0));
        assert!(
            accel.x < 0.0,
            "should pull back towards the origin: {accel}"
        );
        assert!((accel.length() - 9.81).abs() < 1e-3, "{accel}");
    }

    /// The strength is quoted at the radius, so that is where it holds
    /// exactly — which is what makes it an authorable number.
    #[test]
    fn the_strength_is_exact_at_the_radius() {
        let source = PointGravity::default();
        let at_radius = source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 50.0, 0.0));
        assert!((at_radius.length() - 9.81).abs() < 1e-3);

        let further = source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 100.0, 0.0));
        assert!(
            (further.length() - 9.81 / 4.0).abs() < 1e-3,
            "twice the distance should be a quarter the pull, got {}",
            further.length(),
        );
    }

    /// Inside the reference radius the pull holds rather than growing.
    /// Unclamped it goes to infinity at the centre, which launches things
    /// out of the world.
    #[test]
    fn the_pull_does_not_grow_without_bound_near_the_centre() {
        let source = PointGravity::default();
        let close = source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 0.001, 0.0));
        assert!(close.length() <= 9.81 + 1e-3, "{}", close.length());
        assert!(close.is_finite());
    }

    /// Exactly at the centre there is no direction to pull in.
    #[test]
    fn a_body_at_the_centre_is_pulled_nowhere() {
        let accel = PointGravity::default().acceleration_at(Vec3::ZERO, Vec3::ZERO);
        assert_eq!(accel, Vec3::ZERO);
    }

    /// The cutoff is what keeps a galaxy of sources from costing every
    /// body every step.
    #[test]
    fn beyond_the_range_a_source_contributes_nothing() {
        let source = PointGravity {
            range: 100.0,
            ..Default::default()
        };
        assert_eq!(
            source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 101.0, 0.0)),
            Vec3::ZERO,
        );
        assert_ne!(
            source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 99.0, 0.0)),
            Vec3::ZERO,
        );
    }

    #[test]
    fn a_constant_point_source_does_not_fall_off() {
        let source = PointGravity {
            inverse_square: false,
            ..Default::default()
        };
        let near = source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 10.0, 0.0));
        let far = source.acceleration_at(Vec3::ZERO, Vec3::new(0.0, 400.0, 0.0));
        assert!((near.length() - far.length()).abs() < 1e-3);
    }

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

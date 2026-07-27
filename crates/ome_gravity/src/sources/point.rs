//! [`PointGravity`] — a field pulling towards one point.

use glam::Vec3;

use ome_ecs::Reflect;
use ome_ecs::component::Component;

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
}

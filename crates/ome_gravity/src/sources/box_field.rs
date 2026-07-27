//! [`BoxGravity`] — a cube planet, pulling towards its nearest surface.

use glam::Vec3;

use ome_ecs::Reflect;
use ome_ecs::component::Component;

/// A solid box that pulls towards whichever part of its surface is
/// nearest: each face along its own normal, and the edges and corners
/// blending between them.
///
/// # How it works, and why the edges need no special case
///
/// The direction is towards the closest point on the box, which is the
/// point clamped into its half-extents. That single expression produces
/// every behaviour the shape needs:
///
/// - **Over a face**, the closest point is directly below, so the pull is
///   exactly that face's normal — constant across the whole face, which is
///   what makes it walkable.
/// - **Past an edge**, the closest point is *on* the edge, and the
///   direction rotates continuously as a body moves around it. The
///   transition between two faces is not blended, interpolated or
///   special-cased; it is what the arithmetic already says.
/// - **Past a corner**, it points at the corner, and the three faces meet
///   with no seam.
///
/// This is the gradient of the box's signed distance function, which is
/// why it comes out consistent: gravity that follows a surface *is* the
/// gradient of the distance to it.
///
/// # This is not [`super::AreaGravity`]
///
/// An area is a region with one uniform down, acting on what is inside it.
/// This is a solid acting on what is outside it, with a different direction
/// at every point. Same primitive, opposite job.
///
/// # Default
///
/// A 10 m cube at Earth strength, with the corners slightly rounded and a
/// 20 m reach.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct BoxGravity {
    /// Half-extents of the solid, in the entity's local space.
    pub half_extents: Vec3,
    /// Acceleration at the surface, in metres per second squared.
    pub strength: f32,
    /// How gently gravity turns around the edges.
    ///
    /// The box is shrunk by this much before the closest point is taken,
    /// so the direction starts turning this far *before* the edge instead
    /// of at it. Zero is a hard cube; a value equal to the half-extents
    /// collapses the box to its centre and the field becomes a sphere —
    /// so this is the dial between a cube planet and a round one.
    pub rounding: f32,
    /// How far from the surface the field holds at full strength.
    ///
    /// Zero or less means unlimited, and `falloff` then never applies.
    pub range: f32,
    /// How far past `range` the field fades to nothing.
    ///
    /// Without it a body leaving the planet's reach loses its gravity
    /// between one step and the next. Zero for a hard cutoff.
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

    /// The direction towards the surface and the distance to it, or `None`
    /// for a point inside the solid.
    ///
    /// Split out because the direction alone is what a character controller
    /// wants — "which way is down from here" is a question worth asking
    /// without also applying a force.
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
mod tests {
    use super::*;

    /// A hard cube, big enough that the test distances are unambiguous.
    fn cube() -> BoxGravity {
        BoxGravity {
            half_extents: Vec3::splat(10.0),
            rounding: 0.0,
            range: 0.0,
            falloff: 0.0,
            ..Default::default()
        }
    }

    /// The claim the component makes: over a face, gravity is that face's
    /// normal — and the same one everywhere on it, or you could not walk
    /// across it without leaning.
    #[test]
    fn each_face_pulls_along_its_own_normal() {
        let field = cube();
        for (probe, wanted) in [
            (Vec3::new(0.0, 15.0, 0.0), Vec3::NEG_Y),
            (Vec3::new(0.0, -15.0, 0.0), Vec3::Y),
            (Vec3::new(15.0, 0.0, 0.0), Vec3::NEG_X),
            (Vec3::new(-15.0, 0.0, 0.0), Vec3::X),
            (Vec3::new(0.0, 0.0, 15.0), Vec3::NEG_Z),
            (Vec3::new(0.0, 0.0, -15.0), Vec3::Z),
            // Off-centre on the +Y face, still straight down.
            (Vec3::new(9.0, 15.0, -9.0), Vec3::NEG_Y),
        ] {
            let accel = field.acceleration_at_local(probe);
            assert!(
                accel.normalize().abs_diff_eq(wanted, 1e-4),
                "at {probe} the pull was {accel}, wanted {wanted}",
            );
        }
    }

    /// The reason no edge case is written anywhere: the closest-point
    /// direction is continuous, so walking over an edge turns gravity
    /// smoothly instead of flipping it in one step.
    #[test]
    fn gravity_turns_continuously_around_an_edge() {
        let field = cube();
        // A quarter arc around the +X/+Y edge, from clearly over the top
        // face to clearly out from the side one, at a constant 5 m.
        const EDGE: Vec3 = Vec3::new(10.0, 10.0, 0.0);
        const STEPS: u32 = 60;
        let sweep = std::f32::consts::FRAC_PI_2 + 1.2;

        let mut previous: Option<Vec3> = None;
        let mut first = Vec3::ZERO;
        for step in 0..=STEPS {
            let angle = -0.6 + sweep * step as f32 / STEPS as f32;
            let probe = EDGE + Vec3::new(angle.sin(), angle.cos(), 0.0) * 5.0;
            let now = field.acceleration_at_local(probe).normalize();
            match previous {
                None => first = now,
                Some(before) => {
                    let turn = before.dot(now).clamp(-1.0, 1.0).acos().to_degrees();
                    assert!(turn < 10.0, "gravity jumped {turn}° in one step at {probe}");
                }
            }
            previous = Some(now);
        }

        // And it did turn the whole quarter: a field that never moved at
        // all would pass the check above trivially.
        assert!(first.abs_diff_eq(Vec3::NEG_Y, 1e-3), "started at {first}");
        assert!(
            previous.expect("sampled").abs_diff_eq(Vec3::NEG_X, 1e-3),
            "ended at {:?}",
            previous,
        );
    }

    /// Diagonally out from a corner, all three faces are equally near.
    #[test]
    fn a_corner_pulls_along_its_diagonal() {
        let field = cube();
        let accel = field.acceleration_at_local(Vec3::splat(20.0));
        assert!(
            accel
                .normalize()
                .abs_diff_eq(Vec3::splat(-1.0).normalize(), 1e-4),
            "{accel}",
        );
    }

    /// Inside the solid there is no surface to fall towards. A body there
    /// is inside the rock, and inventing a direction for it would be a
    /// force that shoots it out of the planet.
    #[test]
    fn inside_the_solid_nothing_pulls() {
        let field = cube();
        assert_eq!(field.acceleration_at_local(Vec3::ZERO), Vec3::ZERO);
        assert_eq!(field.acceleration_at_local(Vec3::splat(9.0)), Vec3::ZERO);
    }

    /// Rounding equal to the half-extents shrinks the box to its centre,
    /// and the closest-point field around a point *is* a sphere. The dial
    /// runs all the way from cube to planet.
    #[test]
    fn full_rounding_makes_a_sphere() {
        let field = BoxGravity {
            half_extents: Vec3::splat(10.0),
            rounding: 10.0,
            range: 0.0,
            ..Default::default()
        };
        // Over the corner diagonal, a cube would still pull along the
        // diagonal — but so does a sphere, so probe somewhere they differ:
        // over a face, a cube pulls straight down and a sphere pulls at the
        // centre, which from here is the same. Use an oblique point.
        let probe = Vec3::new(4.0, 20.0, 0.0);
        let accel = field.acceleration_at_local(probe);
        assert!(
            accel.normalize().abs_diff_eq(-probe.normalize(), 1e-4),
            "a fully rounded box should pull at its centre: {accel}",
        );
    }

    /// And with no rounding the same probe pulls straight down instead,
    /// which is what makes the previous test mean something.
    #[test]
    fn a_hard_cube_does_not_pull_at_its_centre() {
        let accel = cube().acceleration_at_local(Vec3::new(4.0, 20.0, 0.0));
        assert!(accel.normalize().abs_diff_eq(Vec3::NEG_Y, 1e-4), "{accel}");
    }

    #[test]
    fn the_field_fades_past_its_range() {
        let field = BoxGravity {
            half_extents: Vec3::splat(10.0),
            rounding: 0.0,
            range: 5.0,
            falloff: 10.0,
            ..Default::default()
        };
        // Distances are measured from the surface, not from the centre.
        assert_eq!(field.influence(4.0), 1.0);
        assert!((field.influence(10.0) - 0.5).abs() < 1e-4);
        assert_eq!(field.influence(16.0), 0.0);
        assert_eq!(
            field.acceleration_at_local(Vec3::new(0.0, 30.0, 0.0)),
            Vec3::ZERO,
        );
    }

    /// Zero range is unlimited, or a planet would need its reach retyped
    /// every time it grew.
    #[test]
    fn an_unlimited_field_never_fades() {
        assert_eq!(cube().influence(10_000.0), 1.0);
    }
}

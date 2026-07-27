//! A room with its own down: the box, the reach of its falloff, and which
//! way down is inside it.

use glam::{Mat3, Vec3};

use ome_ecs::hierarchy::GlobalTransform;
use ome_gizmos::{Gizmos, Visualizer};
use ome_gravity::AreaGravity;

use super::{EDGE, FIELD, arrow};

#[derive(Default)]
pub(crate) struct AreaGravityVisualizer;

impl Visualizer<AreaGravity> for AreaGravityVisualizer {
    fn draw(&self, field: &AreaGravity, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let (scale, rotation, origin) = transform.matrix.to_scale_rotation_translation();
        let basis = Mat3::from_quat(rotation);
        // Scaled the way the field itself is: the box is authored in local
        // space, so a scaled entity has a bigger zone.
        let half = field.half_extents.abs() * scale;

        gizmos.wire_obb(origin, basis, half, FIELD);
        // The outer shell is where the field reaches zero. Without it the
        // author sees a hard box and cannot tell a 0.1 m fade from a 5 m one.
        if field.falloff > 0.0 {
            gizmos.wire_obb(origin, basis, half + Vec3::splat(field.falloff), EDGE);
        }

        // `direction` is local, so this is the drawing that shows a rotated
        // zone actually rotated — the exact thing that was invisible.
        let Some(local) = field.direction.try_normalize() else {
            return;
        };
        let world = rotation * local;
        // Four arrows spread across the face the field points away from, so
        // "down" reads as a direction through the volume rather than one
        // stray line at the centre.
        let (a, b) = local.any_orthonormal_pair();
        for (u, v) in [(0.5, 0.5), (0.5, -0.5), (-0.5, 0.5), (-0.5, -0.5)] {
            let inside = (a * u + b * v) * half;
            arrow(
                gizmos,
                origin + rotation * (inside - local * half * 0.5),
                world,
                FIELD,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gizmos::gravity::harness::{draw, reach, shafts};
    use glam::{Mat4, Quat};

    #[test]
    fn an_area_draws_its_box() {
        let field = AreaGravity {
            half_extents: Vec3::splat(5.0),
            falloff: 0.0,
            ..Default::default()
        };
        let corner = Vec3::splat(5.0).length();
        let reach = reach(&draw(&AreaGravityVisualizer, &field, Mat4::IDENTITY));
        assert!(
            (reach - corner).abs() < 0.1,
            "reached {reach}, wanted {corner}",
        );
    }

    /// The falloff is where the field actually ends, and it is invisible
    /// otherwise: a 0.1 m fade and a 5 m fade look identical without it.
    #[test]
    fn an_area_draws_the_reach_of_its_falloff() {
        let with = AreaGravity {
            half_extents: Vec3::splat(5.0),
            falloff: 5.0,
            ..Default::default()
        };
        let without = AreaGravity {
            falloff: 0.0,
            ..with
        };
        assert!(
            reach(&draw(&AreaGravityVisualizer, &with, Mat4::IDENTITY))
                > reach(&draw(&AreaGravityVisualizer, &without, Mat4::IDENTITY)) + 1.0,
        );
    }

    /// `direction` is local, so a rotated zone must draw rotated. This is
    /// the case that had no way of being seen and dropped things sideways
    /// for a reason nobody could point at.
    #[test]
    fn an_area_turns_with_its_entity() {
        let field = AreaGravity::default();
        let turned = Mat4::from_quat(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        let shafts = shafts(&draw(&AreaGravityVisualizer, &field, turned));

        assert!(!shafts.is_empty(), "no arrows were drawn");
        // Local -Y turned a quarter turn about +Z points along +X, the same
        // answer `plugin::gravity_at` gives for the same transform.
        for shaft in shafts {
            assert!(
                shaft.abs_diff_eq(Vec3::X, 1e-3),
                "a rotated zone drew its arrow along {shaft}",
            );
        }
    }

    /// A scaled entity has a bigger zone, the same way the solver scales it.
    #[test]
    fn an_area_scales_with_its_entity() {
        let field = AreaGravity {
            half_extents: Vec3::splat(5.0),
            falloff: 0.0,
            ..Default::default()
        };
        let plain = reach(&draw(&AreaGravityVisualizer, &field, Mat4::IDENTITY));
        let scaled = reach(&draw(
            &AreaGravityVisualizer,
            &field,
            Mat4::from_scale(Vec3::splat(2.0)),
        ));
        assert!((scaled / plain - 2.0).abs() < 0.05, "{plain} then {scaled}");
    }
}

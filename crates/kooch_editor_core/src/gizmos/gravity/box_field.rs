//! A cube planet: the solid, where its edges start to turn, how far it
//! reaches, and one arrow per face along that face's own normal.

use glam::{Mat3, Vec3};

use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_gravity::BoxGravity;

use super::{ARROW, EDGE, FIELD, arrow};

/// The six face normals, in the source's local space.
const FACES: [Vec3; 6] = [
    Vec3::X,
    Vec3::NEG_X,
    Vec3::Y,
    Vec3::NEG_Y,
    Vec3::Z,
    Vec3::NEG_Z,
];

#[derive(Default)]
pub(crate) struct BoxGravityVisualizer;

impl Visualizer<BoxGravity> for BoxGravityVisualizer {
    fn draw(&self, field: &BoxGravity, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let (scale, rotation, origin) = transform.matrix.to_scale_rotation_translation();
        let basis = Mat3::from_quat(rotation);
        let half = field.half_extents.abs() * scale;

        gizmos.wire_obb(origin, basis, half, FIELD);

        // The shrunk box is not decoration: it is what the closest point is
        // taken against, so it is literally where gravity begins to turn.
        // Drawing it is the only way `rounding` is anything but a number.
        let rounding = field.rounding.max(0.0);
        if rounding > 0.0 {
            let inner = (half - Vec3::splat(rounding)).max(Vec3::ZERO);
            gizmos.wire_obb(origin, basis, inner, EDGE);
        }

        // How far the pull reaches, measured from the surface. An inflated
        // box overstates the corners slightly — the true iso-surface is
        // rounded there — but it answers "does this planet reach that
        // platform", which is the question being asked.
        if field.range > 0.0 {
            let reach = half + Vec3::splat(field.range + field.falloff.max(0.0));
            gizmos.wire_obb(origin, basis, reach, EDGE);
        }

        // One arrow per face, landing on the face centre along that face's
        // own normal. This is the whole claim the component makes, and
        // there is nothing else in the editor that would show it.
        for normal in FACES {
            let face = normal * half;
            let base = face + normal * ARROW;
            arrow(gizmos, origin + rotation * base, rotation * -normal, FIELD);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gizmos::gravity::harness::{draw, reach, shafts};
    use glam::{Mat4, Quat};

    fn cube() -> BoxGravity {
        BoxGravity {
            half_extents: Vec3::splat(10.0),
            rounding: 0.0,
            range: 0.0,
            falloff: 0.0,
            ..Default::default()
        }
    }

    /// The claim: each face pulls along its own normal. Six arrows, one per
    /// face, each pointing at the solid.
    #[test]
    fn every_face_gets_an_arrow_along_its_own_normal() {
        let shafts = shafts(&draw(&BoxGravityVisualizer, &cube(), Mat4::IDENTITY));
        assert_eq!(shafts.len(), 6, "expected one arrow per face");
        for normal in FACES {
            assert!(
                shafts.iter().any(|s| s.abs_diff_eq(-normal, 1e-3)),
                "no arrow pulls along {normal}; got {shafts:?}",
            );
        }
    }

    #[test]
    fn the_solid_is_drawn_at_its_half_extents() {
        let corner = Vec3::splat(10.0).length();
        let reach = reach(&draw(&BoxGravityVisualizer, &cube(), Mat4::IDENTITY));
        // Plus the arrows, which stand `ARROW` off each face — shorter than
        // the corner diagonal, so the corner still sets the reach.
        assert!(
            (reach - corner).abs() < 0.1,
            "reached {reach}, wanted {corner}",
        );
    }

    /// `rounding` decides where gravity starts turning, and without the
    /// inner box on screen it is a number with nothing to check it against.
    #[test]
    fn rounding_draws_the_box_it_actually_clamps_against() {
        let hard = draw(&BoxGravityVisualizer, &cube(), Mat4::IDENTITY);
        let rounded = draw(
            &BoxGravityVisualizer,
            &BoxGravity {
                rounding: 4.0,
                ..cube()
            },
            Mat4::IDENTITY,
        );
        assert!(
            rounded.len() > hard.len(),
            "rounding drew nothing extra: {} then {}",
            hard.len(),
            rounded.len(),
        );
    }

    /// A planet with a reach has to show it, or "does this pull that
    /// platform" is unanswerable without running the game.
    #[test]
    fn the_reach_is_drawn_when_the_field_is_limited() {
        let limited = BoxGravity {
            range: 20.0,
            falloff: 5.0,
            ..cube()
        };
        let far = reach(&draw(&BoxGravityVisualizer, &limited, Mat4::IDENTITY));
        let near = reach(&draw(&BoxGravityVisualizer, &cube(), Mat4::IDENTITY));
        assert!(far > near + 20.0, "{near} then {far}");
    }

    /// Turning the planet turns its faces, so the arrows have to follow —
    /// the same round trip through the entity's rotation the solver makes.
    #[test]
    fn the_faces_turn_with_the_entity() {
        let turned = Mat4::from_quat(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        let shafts = shafts(&draw(&BoxGravityVisualizer, &cube(), turned));

        // Local +X turned a quarter turn about +Z points along +Y, so the
        // arrow onto that face now pulls along -Y.
        assert!(
            shafts.iter().any(|s| s.abs_diff_eq(Vec3::NEG_Y, 1e-3)),
            "no arrow follows the rotated +X face: {shafts:?}",
        );
    }

    #[test]
    fn the_solid_scales_with_the_entity() {
        let plain = reach(&draw(&BoxGravityVisualizer, &cube(), Mat4::IDENTITY));
        let scaled = reach(&draw(
            &BoxGravityVisualizer,
            &cube(),
            Mat4::from_scale(Vec3::splat(2.0)),
        ));
        assert!((scaled / plain - 2.0).abs() < 0.05, "{plain} then {scaled}");
    }
}

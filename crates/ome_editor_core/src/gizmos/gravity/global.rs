//! A uniform field: parallel arrows, because one arrow is a vector and a
//! field is the same vector everywhere.

use glam::Vec3;

use ome_ecs::hierarchy::GlobalTransform;
use ome_gizmos::{Gizmos, Visualizer};
use ome_gravity::GlobalGravity;

use super::{FIELD, arrow};

#[derive(Default)]
pub(crate) struct GlobalGravityVisualizer;

impl Visualizer<GlobalGravity> for GlobalGravityVisualizer {
    fn draw(&self, field: &GlobalGravity, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let Some(direction) = field.acceleration.try_normalize() else {
            return;
        };
        let origin = transform.matrix.to_scale_rotation_translation().2;

        // World space on purpose, and there is a test that pins it:
        // `acceleration` is a world vector, so turning the entity must not
        // turn the field. Deriving the arrows from the entity's basis would
        // be the obvious thing and would quietly make the component lie.
        let (a, b) = direction.any_orthonormal_pair();
        const SPREAD: f32 = 0.8;
        for offset in [
            Vec3::ZERO,
            (a + b) * SPREAD,
            (a - b) * SPREAD,
            (-a + b) * SPREAD,
            (-a - b) * SPREAD,
        ] {
            arrow(gizmos, origin + offset, direction, FIELD);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gizmos::gravity::harness::{draw, shaft};
    use glam::{Mat4, Quat};

    #[test]
    fn a_uniform_field_draws_along_its_acceleration() {
        let field = GlobalGravity::default();
        let segments = draw(&GlobalGravityVisualizer, &field, Mat4::IDENTITY);
        assert!(shaft(&segments).abs_diff_eq(Vec3::NEG_Y, 1e-3));
    }

    /// `acceleration` is a world vector. Deriving the arrows from the
    /// entity's basis would be the natural thing to write and would make
    /// the gizmo disagree with the solver the moment anyone rotated it.
    #[test]
    fn a_uniform_field_does_not_turn_with_its_entity() {
        let field = GlobalGravity::default();
        let turned = Mat4::from_quat(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        assert!(
            shaft(&draw(&GlobalGravityVisualizer, &field, turned)).abs_diff_eq(Vec3::NEG_Y, 1e-3)
        );
    }

    #[test]
    fn a_degenerate_field_draws_nothing() {
        let field = GlobalGravity {
            acceleration: Vec3::ZERO,
        };
        assert!(draw(&GlobalGravityVisualizer, &field, Mat4::IDENTITY).is_empty());
    }
}

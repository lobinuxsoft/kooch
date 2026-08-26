//! A uniform field: parallel arrows, because one arrow is a vector and a
//! field is the same vector everywhere.

use glam::Vec3;

use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_gravity::GlobalGravity;

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
mod tests;

//! A planet: the sphere where the strength is exact, the sphere where it
//! stops, and arrows saying the pull is inward.

use glam::{Mat3, Vec3};

use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_gravity::PointGravity;

use super::{ARROW, EDGE, FIELD, arrow};

#[derive(Default)]
pub(crate) struct PointGravityVisualizer;

impl Visualizer<PointGravity> for PointGravityVisualizer {
    fn draw(&self, field: &PointGravity, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let (_, rotation, origin) = transform.matrix.to_scale_rotation_translation();
        let basis = Mat3::from_quat(rotation);

        // `radius` is where `strength` holds exactly — the one distance in
        // the component that means something an author can check.
        if field.radius > 0.0 {
            gizmos.wire_sphere(origin, basis, field.radius, FIELD);
        }
        // Zero or less is unlimited, and there is no sphere for infinity.
        if field.range > 0.0 {
            gizmos.wire_sphere(origin, basis, field.range, EDGE);
        }

        // Six arrows pointing *in*. This is the difference between a planet
        // and the world vector, and it is the whole reason #624 exists.
        let surface = match field.radius > 0.0 {
            true => field.radius,
            false => ARROW,
        };
        for axis in [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ] {
            arrow(gizmos, origin + axis * surface, -axis, FIELD);
        }
    }
}

#[cfg(test)]
mod tests;

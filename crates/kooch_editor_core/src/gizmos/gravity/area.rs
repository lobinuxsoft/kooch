//! A room with its own down: the box, the reach of its falloff, and which
//! way down is inside it.

use glam::{Mat3, Vec3};

use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_gravity::AreaGravity;

use super::{EDGE, FIELD, arrow};

#[derive(Default)]
pub(crate) struct AreaGravityVisualizer;

impl Visualizer<AreaGravity> for AreaGravityVisualizer {
    fn draw(&self, field: &AreaGravity, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let (_, rotation, origin) = transform.matrix.to_scale_rotation_translation();
        // No scale, because the field has none: its space is rigid, so
        // `half_extents` and `falloff` are already metres.
        let basis = Mat3::from_quat(rotation);
        let half = field.half_extents.abs();

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
                origin + basis * (inside - local * half * 0.5),
                world,
                FIELD,
            );
        }
    }
}

#[cfg(test)]
mod tests;

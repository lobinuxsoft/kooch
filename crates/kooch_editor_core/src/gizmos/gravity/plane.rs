//! A floor: the surface, the height its pull holds to, and where it dies.

use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_gravity::PlaneGravity;

use super::{EDGE, FIELD, arrow};

/// How wide a patch of an unbounded plane to draw.
///
/// Any number here is a lie — the plane does not stop. It is a fixed size
/// so the *vertical* spacing, which is the part that means something, is
/// what changes when the author edits the component.
const PATCH: f32 = 8.0;

#[derive(Default)]
pub(crate) struct PlaneGravityVisualizer;

impl Visualizer<PlaneGravity> for PlaneGravityVisualizer {
    fn draw(&self, field: &PlaneGravity, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let (_, rotation, origin) = transform.matrix.to_scale_rotation_translation();
        let Some(local) = field.normal.try_normalize() else {
            return;
        };
        let Some(normal) = (rotation * local).try_normalize() else {
            return;
        };

        // Metres along the world normal: the field's space is rigid, so
        // a scaled entity does not reach further. See `local_space`.
        let step = |height: f32| normal * height;

        // The surface. `wire_halfspace` marks which side is active with a
        // stub along the normal, which is the same thing it means for a
        // half-space collider: this is the side the field acts on.
        gizmos.wire_halfspace(origin, normal, PATCH, FIELD);

        // The two heights an author edits. Without them a 5 m reach and a
        // 500 m one are the same picture.
        if field.range > 0.0 {
            gizmos.wire_halfspace(origin + step(field.range), normal, PATCH, FIELD);
            if field.falloff > 0.0 {
                let outer = field.range + field.falloff;
                gizmos.wire_halfspace(origin + step(outer), normal, PATCH, EDGE);
            }
        }

        // Pointing down, at the height where the pull is still full. An
        // arrow along the normal would read as a repulsor.
        let (u, v) = normal.any_orthonormal_pair();
        let raised = match field.range > 0.0 {
            true => step(field.range),
            false => normal * PATCH,
        };
        for (a, b) in [(0.5, 0.5), (0.5, -0.5), (-0.5, 0.5), (-0.5, -0.5)] {
            let across = (u * a + v * b) * PATCH;
            arrow(gizmos, origin + across + raised, -normal, FIELD);
        }
    }
}

#[cfg(test)]
mod tests;

//! A cube planet: the solid, where its edges start to turn, how far it
//! reaches, and one arrow per face along that face's own normal.

use glam::{Mat3, Vec3};

use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_gravity::BoxGravity;

use super::{ARROW, EDGE, FADE, FIELD, arrow};

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
        let (_, rotation, origin) = transform.matrix.to_scale_rotation_translation();
        // No scale, because the field has none: its space is rigid, so
        // every extent here is already metres. See `local_space`.
        let basis = Mat3::from_quat(rotation);
        let half = field.half_extents.abs();

        gizmos.wire_obb(origin, basis, half, FIELD);

        // The shrunk box is not decoration: it is what the closest point is
        // taken against, so it is literally where gravity begins to turn.
        // Drawing it is the only way `rounding` is anything but a number.
        let rounding = field.rounding.max(0.0);
        if rounding > 0.0 {
            let inner = (half - Vec3::splat(rounding)).max(Vec3::ZERO);
            gizmos.wire_obb(origin, basis, inner, EDGE);
        }

        // How far the pull reaches, measured from the surface, and each face on its own. An
        // inflated box overstates the corners slightly — the true iso-surface is rounded there —
        // but it answers "does this planet reach that platform", which is the question being asked.
        //
        // 🔴 Two shells, not one. A single one drawn at `range + falloff` showed where gravity
        // ENDS and nothing about where it starts to fade: the band between them is the whole of
        // `falloff`, and without it an author is typing a number they cannot see (#1324).
        let (ahead, behind) = (field.range_positive, field.range_negative);
        if ahead.min_element() > 0.0 && behind.min_element() > 0.0 {
            let mut shell = |out: Vec3, back: Vec3, colour: Vec3| {
                let (max, min) = (half + out, -(half + back));
                let centre = origin + basis * ((max + min) * 0.5);
                gizmos.wire_obb(centre, basis, (max - min) * 0.5, colour);
            };
            shell(ahead, behind, EDGE);
            let fade = field.falloff.max(0.0);
            if fade > 0.0 {
                shell(ahead + Vec3::splat(fade), behind + Vec3::splat(fade), FADE);
            }
        }

        // One arrow per face, landing on the face centre along that face's
        // own normal. This is the whole claim the component makes, and
        // there is nothing else in the editor that would show it.
        for normal in FACES {
            let face = basis * (normal * half);
            arrow(
                gizmos,
                origin + face + rotation * normal * ARROW,
                rotation * -normal,
                FIELD,
            );
        }
    }
}

#[cfg(test)]
mod tests;

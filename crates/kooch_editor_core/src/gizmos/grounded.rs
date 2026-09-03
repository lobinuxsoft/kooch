//! What the controller actually found, beside what it was asking for.
//!
//! [`CharacterVisualizer`](super::character::CharacterVisualizer) draws
//! the numbers in the Inspector: where the body *should* ride, how far it
//! *may* look. This draws the answer the sweep came back with. Together
//! they are the debug view — a gap that does not match the ride height,
//! or a normal that is not the ramp you are standing on, is visible
//! rather than deduced.

use glam::Vec3;

use kooch_character::Grounded;
use kooch_core::resource::Resources;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};

/// Green while standing, matching the controller's own rest marks.
const STANDING: Vec3 = Vec3::new(0.35, 0.85, 0.45);

/// Amber for ground that was found and cannot be stood on — a wall the
/// spring is still pushing off.
const REFUSED: Vec3 = Vec3::new(0.98, 0.68, 0.22);

/// Long enough to read the slope off at a glance.
const NORMAL: f32 = 1.0;

#[derive(Default)]
pub(crate) struct GroundedVisualizer;

impl Visualizer<Grounded> for GroundedVisualizer {
    fn draw(&self, found: &Grounded, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        draw_at(found, transform, Vec3::Y, gizmos);
    }

    fn draw_with(
        &self,
        found: &Grounded,
        transform: &GlobalTransform,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        let origin = transform.matrix.to_scale_rotation_translation().2;
        draw_at(
            found,
            transform,
            kooch_gravity::gravity_up(resources, origin),
            gizmos,
        );
    }
}

fn draw_at(found: &Grounded, transform: &GlobalTransform, up: Vec3, gizmos: &mut Gizmos<'_>) {
    // Nothing under it. Drawing a contact at the origin would read as
    // ground at your feet, which is the opposite of the truth.
    let Some(normal) = found.normal.try_normalize() else {
        return;
    };
    let up = up.normalize_or(Vec3::Y);
    let origin = transform.matrix.to_scale_rotation_translation().2;
    let colour = match found.standing {
        true => STANDING,
        false => REFUSED,
    };

    // Where the sweep stopped, measured the way the spring measures it.
    let contact = origin - up * found.distance;
    let (u, v) = normal.any_orthonormal_pair();
    gizmos.wire_circle(contact, u, v, 0.25, colour);
    gizmos.line(origin, contact, colour);

    // The surface normal is the slope, and the angle between it and the
    // arrow the controller draws is the whole question of "why will it
    // not let me up here".
    let (a, b) = normal.any_orthonormal_pair();
    gizmos.arrow(contact, contact + normal * NORMAL, a, b, colour);
}

#[cfg(test)]
mod tests;

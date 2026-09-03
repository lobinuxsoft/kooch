//! Where gameplay is steering, beside where the body actually looks.
//!
//! The two are never the same for long — the body turns at
//! [`turn_speed`](kooch_character::CharacterController::turn_speed) and
//! the steering moves with the camera — so the useful thing to see is
//! the gap between them. A character that will not turn draws one arrow
//! that moves and one that does not.

use glam::Vec3;

use kooch_character::Facing;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};

/// Cyan: an input, not a measurement — every other character mark is a
/// measurement.
const STEERED: Vec3 = Vec3::new(0.35, 0.8, 0.95);

/// The same hue, darker, for where the body has actually got to.
const LOOKING: Vec3 = Vec3::new(0.18, 0.42, 0.55);

/// Long enough to read a heading off at a glance.
const ARROW: f32 = 1.5;

#[derive(Default)]
pub(crate) struct FacingVisualizer;

impl Visualizer<Facing> for FacingVisualizer {
    fn draw(&self, facing: &Facing, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        draw_at(facing, transform, Vec3::Y, gizmos);
    }

    fn draw_with(
        &self,
        facing: &Facing,
        transform: &GlobalTransform,
        _entity: Entity,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        let origin = transform.matrix.to_scale_rotation_translation().2;
        draw_at(
            facing,
            transform,
            kooch_gravity::gravity_up(resources, origin),
            gizmos,
        );
    }
}

fn draw_at(facing: &Facing, transform: &GlobalTransform, up: Vec3, gizmos: &mut Gizmos<'_>) {
    let (_, rotation, origin) = transform.matrix.to_scale_rotation_translation();
    let up = up.normalize_or(Vec3::Y);

    // Flattened the same way the controller flattens it, or the arrow
    // would point into the slope the body is turning along.
    let flat = |direction: Vec3| (direction - up * direction.dot(up)).try_normalize();

    if let Some(looking) = flat(rotation * Vec3::NEG_Z) {
        arrow(gizmos, origin, looking, ARROW * 0.8, LOOKING);
    }
    // Nothing steered is not "steered at zero": the controller keeps the
    // heading it has, so drawing an arrow would invent an intent.
    if let Some(steered) = flat(facing.direction) {
        arrow(gizmos, origin, steered, ARROW, STEERED);
    }
}

/// A shaft with two barbs, in the plane the character turns in.
fn arrow(gizmos: &mut Gizmos<'_>, origin: Vec3, direction: Vec3, length: f32, colour: Vec3) {
    let tip = origin + direction * length;
    gizmos.line(origin, tip, colour);
    let across = direction
        .cross(origin.normalize_or(Vec3::Y))
        .normalize_or(direction.any_orthonormal_vector());
    let barb = length * 0.2;
    gizmos.line(tip, tip - direction * barb + across * barb, colour);
    gizmos.line(tip, tip - direction * barb - across * barb, colour);
}

#[cfg(test)]
mod tests;

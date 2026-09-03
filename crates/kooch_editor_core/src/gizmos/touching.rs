//! The wall the character found, and which way it faces.
//!
//! `Grounded`'s horizontal twin, and drawn like it: the contact where
//! the probe stopped, and an arrow along the normal. A wall slide that
//! refuses to start is either a wall nobody found or a normal pointing
//! somewhere unexpected, and those look identical in the Inspector.

use glam::Vec3;

use kooch_character::Touching;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};

/// Blue, so it cannot be mistaken for the ground marks beneath it.
const WALL: Vec3 = Vec3::new(0.4, 0.6, 0.95);

/// Long enough to read the facing off at a glance.
const ARROW: f32 = 0.8;

#[derive(Default)]
pub(crate) struct TouchingVisualizer;

impl Visualizer<Touching> for TouchingVisualizer {
    fn draw(&self, found: &Touching, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        draw_at(found, transform, Vec3::Y, gizmos);
    }

    fn draw_with(
        &self,
        found: &Touching,
        transform: &GlobalTransform,
        _entity: Entity,
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

fn draw_at(found: &Touching, transform: &GlobalTransform, up: Vec3, gizmos: &mut Gizmos<'_>) {
    // Nothing found draws nothing. A contact at the origin would read as
    // a wall the character is inside.
    if !found.wall {
        return;
    }
    let Some(normal) = found.normal.try_normalize() else {
        return;
    };
    let up = up.normalize_or(Vec3::Y);
    let origin = transform.matrix.to_scale_rotation_translation().2;
    // Where the probe stopped, which is not where the body is.
    let contact = origin - normal * found.distance.max(0.0);

    let across = normal
        .cross(up)
        .normalize_or(normal.any_orthonormal_vector());
    gizmos.wire_circle(contact, across, normal.cross(across), 0.3, WALL);
    gizmos.line(contact, contact + normal * ARROW, WALL);
}

#[cfg(test)]
mod tests;

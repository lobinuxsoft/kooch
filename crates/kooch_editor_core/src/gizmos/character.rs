//! The floating capsule's numbers, which are all invisible until
//! something falls through the floor.
//!
//! A collider that is the wrong size shows up the moment anything rests
//! on it. A character controller has no surface of its own: the ride
//! height is a gap that is *supposed* to be empty, the probe is a sweep
//! that leaves no trace, and the slope limit is the difference between a
//! ramp and a wall with nothing between them.
//!
//! # The one mistake this exists to catch
//!
//! `ride_height` is measured from the body's origin, so it has to clear
//! the collider's own reach downward — otherwise the spring asks for a
//! height the geometry cannot occupy and the capsule quietly rests on
//! the floor instead of floating. That is a number-versus-number
//! comparison nobody makes in their head, and it is drawn in amber.

use glam::{Mat3, Vec3};

use kooch_character::CharacterController;
use kooch_core::resource::Resources;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};

/// Amber: the same "this is wrong and you cannot see why" the stale-bake
/// warning uses.
const REFUSED: Vec3 = Vec3::new(0.98, 0.68, 0.22);

/// Green, like the colliders it stands beside.
const HELD: Vec3 = Vec3::new(0.35, 0.85, 0.45);

/// The same hue, darker, for the reach rather than the rest position.
const EDGE: Vec3 = Vec3::new(0.18, 0.45, 0.24);

#[derive(Default)]
pub(crate) struct CharacterVisualizer;

impl Visualizer<CharacterController> for CharacterVisualizer {
    fn draw(
        &self,
        controller: &CharacterController,
        transform: &GlobalTransform,
        gizmos: &mut Gizmos<'_>,
    ) {
        // Without `Resources` there is no field to ask, so up is world
        // up — which is also what the controller would use.
        draw_at(controller, transform, Vec3::Y, gizmos);
    }

    fn draw_with(
        &self,
        controller: &CharacterController,
        transform: &GlobalTransform,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        let origin = transform.matrix.to_scale_rotation_translation().2;
        // The same answer the controller will get, not the entity's own
        // rotation: a capsule knocked flat still has the field's up.
        let up = kooch_gravity::gravity_up(resources, origin);
        draw_at(controller, transform, up, gizmos);
    }
}

fn draw_at(
    controller: &CharacterController,
    transform: &GlobalTransform,
    up: Vec3,
    gizmos: &mut Gizmos<'_>,
) {
    let origin = transform.matrix.to_scale_rotation_translation().2;
    let up = up.normalize_or(Vec3::Y);
    let (u, v) = up.any_orthonormal_pair();
    let basis = Mat3::from_cols(u, up, v);
    let radius = controller.probe_radius.max(0.01);

    // Where the body is held. The disc is the plane the origin rides in,
    // and the line down to it is the gap itself.
    let rest = origin - up * controller.ride_height;
    gizmos.wire_circle(rest, u, v, radius, HELD);
    gizmos.line(origin, rest, HELD);

    // How far it looks. Past the ride height this is how far it can drop
    // before it counts as falling; short of it, the probe can never
    // reach the floor it is standing on.
    let end = origin - up * controller.probe.max(0.0);
    gizmos.wire_sphere(end, basis, radius, EDGE);

    // A probe that stops before the rest position can never find ground.
    // Drawn rather than explained, because the two numbers are three
    // fields apart in the Inspector.
    if controller.probe < controller.ride_height {
        gizmos.line(end, rest, REFUSED);
        gizmos.wire_circle(rest, u, v, radius * 1.4, REFUSED);
    }

    // The slope limit, as the cone it is. "Why can I not walk up this
    // ramp" has no answer in a number.
    let slope = controller.max_slope.clamp(0.0, 89.0).to_radians();
    let (sin, cos) = slope.sin_cos();
    for step in 0..8 {
        let turn = step as f32 / 8.0 * std::f32::consts::TAU;
        let across = u * turn.cos() + v * turn.sin();
        let edge = (up * cos + across * sin) * radius * 2.0;
        gizmos.line(rest, rest + edge, EDGE);
    }
    gizmos.wire_circle(
        rest + up * cos * radius * 2.0,
        u,
        v,
        sin * radius * 2.0,
        EDGE,
    );
}

#[cfg(test)]
mod tests;

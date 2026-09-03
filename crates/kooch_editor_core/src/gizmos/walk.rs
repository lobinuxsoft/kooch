//! The velocity being asked for, beside the one the body has.
//!
//! Both are invisible. A character that will not stop and one that is
//! merely slow look identical standing still, and the difference is
//! whether the goal went to zero — which is a number in a resource
//! nobody can open.

use glam::Vec3;

use kooch_character::Walk;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_physics::plugin::{PhysicsWorld, SolverBody};

/// Magenta: what the controller decided to want.
const GOAL: Vec3 = Vec3::new(0.95, 0.45, 0.85);

/// The same hue, darker, for the velocity it actually has.
const ACTUAL: Vec3 = Vec3::new(0.5, 0.2, 0.45);

/// Drawn a metre up, clear of the ride-height marks.
const ABOVE: f32 = 1.0;

#[derive(Default)]
pub(crate) struct WalkVisualizer;

impl Visualizer<Walk> for WalkVisualizer {
    fn draw(&self, walk: &Walk, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        draw_at(walk, transform, Vec3::Y, None, None, gizmos);
    }

    fn draw_with(
        &self,
        walk: &Walk,
        transform: &GlobalTransform,
        entity: Entity,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        let origin = transform.matrix.to_scale_rotation_translation().2;
        let up = kooch_gravity::gravity_up(resources, origin);
        let goal = resources
            .get::<kooch_character::plugin::walk::WalkGoals>()
            .and_then(|goals| goals.of(entity));
        let actual = resources.get::<PhysicsWorld>().and_then(|world| {
            let body = kooch_ecs::query::Query::<&SolverBody>::new(resources).get(entity)?;
            world.linear_velocity(*body)
        });
        draw_at(walk, transform, up, goal, actual, gizmos);
    }
}

fn draw_at(
    walk: &Walk,
    transform: &GlobalTransform,
    up: Vec3,
    goal: Option<Vec3>,
    actual: Option<Vec3>,
    gizmos: &mut Gizmos<'_>,
) {
    let up = up.normalize_or(Vec3::Y);
    let origin = transform.matrix.to_scale_rotation_translation().2 + up * ABOVE;
    let (u, v) = up.any_orthonormal_pair();
    let speed = walk.max_speed.max(0.01);

    // The top speed, as the circle both arrows live inside. An arrow
    // with nothing to be long *against* says nothing.
    gizmos.wire_circle(origin, u, v, 1.0, ACTUAL);

    // Scaled to that circle rather than drawn in metres per second: at
    // walking pace a true-length arrow is longer than the level.
    let mut draw = |velocity: Option<Vec3>, colour| {
        let Some(velocity) = velocity else { return };
        let across = velocity - up * velocity.dot(up);
        if across.length() < 1e-3 {
            return;
        }
        gizmos.line(origin, origin + across / speed, colour);
    };
    draw(actual, ACTUAL);
    draw(goal, GOAL);
}

#[cfg(test)]
mod tests;

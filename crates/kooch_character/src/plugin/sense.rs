//! What is under the character, and what is in front of it.
//!
//! One place casts, everybody reads. Two systems each asking the world
//! the same question is two systems that can disagree about the answer,
//! and this project has already paid for that once: a jump and a walk
//! cast their own ground rays and both were wrong in the same way.

use glam::Vec3;

use kooch_physics::backend::{CollisionShape, QueryFilter, ShapeAt};
use kooch_physics::plugin::PhysicsWorld;

use crate::controller::CharacterController;

/// What a downward probe came back with.
pub struct Under {
    pub point: Vec3,
    pub normal: Vec3,
    /// Ground to stand on: walkable, or a step with a ledge to arrive
    /// at. A wall is neither.
    pub footing: Footing,
}

/// What the surface under the character is *for*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Footing {
    /// Walkable: the spring holds the body over it.
    Ground,
    /// Too steep to walk, but there is a ledge within `step_height`.
    /// The spring holds here too — that lift *is* how a step is climbed.
    Step,
    /// Too steep, and nothing to arrive at. The spring lets go, so
    /// gravity takes the character back down it.
    Wall,
}

impl Footing {
    /// Whether the spring should hold the body up over this.
    pub fn holds(self) -> bool {
        !matches!(self, Self::Wall)
    }

    /// Whether it counts as standing — a step is something you are
    /// getting *over*, not something you are stood on.
    pub fn stands(self) -> bool {
        matches!(self, Self::Ground)
    }
}

/// Sweeps down and classifies what it found.
pub fn under(
    world: &PhysicsWorld,
    controller: &CharacterController,
    position: Vec3,
    up: Vec3,
    filter: QueryFilter,
) -> Option<Under> {
    let probe = CollisionShape::Sphere {
        radius: controller
            .probe_radius
            .max(kooch_physics::backend::MIN_EXTENT),
    };
    let hit = world.sweep(
        ShapeAt::new(&probe, position),
        -up,
        controller.probe.max(0.0),
        filter,
    )?;

    let footing = match controller.stands_on(hit.normal, up) {
        true => Footing::Ground,
        false => match ledge(world, controller, hit.point, hit.normal, up, filter) {
            true => Footing::Step,
            false => Footing::Wall,
        },
    };
    Some(Under {
        point: hit.point,
        normal: hit.normal,
        footing,
    })
}

/// Whether there is walkable ground within `step_height` just past a
/// surface too steep to walk.
///
/// Dropped from above and just beyond the contact, so a step's tread is
/// found and more of the same ramp is not. The offset is the probe's own
/// radius: any less and the ray comes back down the near side of the
/// riser it is trying to see over.
fn ledge(
    world: &PhysicsWorld,
    controller: &CharacterController,
    point: Vec3,
    normal: Vec3,
    up: Vec3,
    filter: QueryFilter,
) -> bool {
    let into = normal - up * normal.dot(up);
    let Some(into) = into.try_normalize() else {
        return false;
    };
    let step = controller.step_height.max(0.0);
    let over = point - into * controller.probe_radius.max(0.05) + up * (step + SKIN);
    match world.raycast_where(over, -up, step + SKIN * 2.0, filter) {
        Some(hit) => controller.stands_on(hit.normal, up),
        None => false,
    }
}

/// Sweeps for the nearest wall ahead or to either side.
///
/// Three sweeps, and the sides are not optional. A probe that only looks
/// where the character is going never finds the wall it is running
/// *along* — which is the one thing a wall run is about, and it meant a
/// character steering parallel to a wall never saw it at all.
///
/// `along` is where the character is steering rather than where it is
/// moving: a body pressed against a wall has almost no velocity into it,
/// which is exactly when a wall slide needs to know the wall is there.
pub fn beside(
    world: &PhysicsWorld,
    controller: &CharacterController,
    position: Vec3,
    along: Vec3,
    up: Vec3,
    filter: QueryFilter,
) -> Option<(Vec3, f32)> {
    let flat = along - up * along.dot(up);
    let forward = flat.try_normalize()?;
    let side = forward.cross(up);
    [forward, side, -side]
        .into_iter()
        .filter_map(|direction| ahead(world, controller, position, direction, up, filter))
        .min_by(|(_, near), (_, far)| near.total_cmp(far))
}

/// One sweep, in one direction.
pub fn ahead(
    world: &PhysicsWorld,
    controller: &CharacterController,
    position: Vec3,
    along: Vec3,
    up: Vec3,
    filter: QueryFilter,
) -> Option<(Vec3, f32)> {
    let direction = along.try_normalize()?;
    let probe = CollisionShape::Sphere {
        radius: controller
            .probe_radius
            .max(kooch_physics::backend::MIN_EXTENT),
    };
    let hit = world.sweep(
        ShapeAt::new(&probe, position),
        direction,
        controller.reach.max(0.0),
        filter,
    )?;
    // A wall is what it cannot walk on. A ramp ahead is not a wall, or
    // every slope would read as one from a metre away.
    match controller.stands_on(hit.normal, up) {
        true => None,
        false => Some((hit.normal, hit.t)),
    }
}

/// Room for floating point either side of the step, in metres.
const SKIN: f32 = 0.02;

#[cfg(test)]
mod tests;

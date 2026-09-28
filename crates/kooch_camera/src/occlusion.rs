//! [`CameraCollision`] — keeping a vcam's camera out of walls (#1251).
//!
//! A spring arm walks through geometry: the pose is planned from the target and the framing, and
//! nothing asks whether the way is clear. This sweeps the camera's own size from the target to where
//! the rig wants the camera, and stops the arm at the first thing in the way — Cinemachine's
//! Deoccluder with its "pull forward" strategy, and Phantom Camera's spring arm.

use std::collections::HashMap;

use glam::Vec3;
use kooch_core::resource::Resources;
use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::FieldRange;

/// Keeps the camera of the vcam it sits on out of geometry. Beside a [`VirtualCamera`]; without the
/// engine's `physics` feature it is authored and does nothing.
///
/// [`VirtualCamera`]: crate::VirtualCamera
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct CameraCollision {
    /// Off lets the arm through walls, as it was.
    pub enabled: bool,
    /// Which collision groups stop the camera. Untick what a camera should see through — glass,
    /// foliage.
    #[reflect(layers)]
    pub groups: u32,
    /// The camera's own size, in metres: it is a sphere this wide that is swept, since a ray slips
    /// through a gap the camera does not fit through.
    #[reflect(range = RADIUS_RANGE)]
    pub radius: f32,
    /// Obstacles closer to the target than this are ignored, in metres — the target's own colliders,
    /// a hat, a sword. It is also as close as the camera ever comes.
    #[reflect(range = DISTANCE_RANGE)]
    pub min_distance: f32,
    /// How fast the camera gets back out once the way clears, in seconds: a hundredth of what is
    /// left after that long, the same easing everything else in the rig uses. Cinemachine's
    /// Deoccluder `Damping`.
    ///
    /// 🔴 Not a duration. The way clears a little at a time and the unobstructed arm moves with the
    /// player, so the goal moves every frame — and a tween with a duration restarts on a moving
    /// goal and steps (#1336, #1339).
    #[reflect(range = RETURN_RANGE, alias = "return_duration, return_time")]
    pub damping: f32,
    /// The same, going in. Zero is immediate, which is what a camera wants: one that eased into a
    /// wall would show the inside of it. Cinemachine's `DampingWhenOccluded`.
    #[reflect(range = RETURN_RANGE)]
    pub damping_when_occluded: f32,
}

const RADIUS_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 2.0,
    step: 0.01,
};

const DISTANCE_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 5.0,
    step: 0.05,
};

const RETURN_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 3.0,
    step: 0.05,
};

impl Default for CameraCollision {
    fn default() -> Self {
        Self {
            enabled: true,
            groups: u32::MAX,
            radius: 0.2,
            min_distance: 0.5,
            damping: 0.35,
            damping_when_occluded: 0.0,
        }
    }
}

impl Component for CameraCollision {}

/// Per vcam, what the arm carries between frames. Runtime state, rebuilt from the vcams seen each
/// frame.
#[derive(Debug, Clone, Default)]
pub struct Arms(HashMap<Entity, Arm>);

#[derive(Debug, Clone, Copy)]
struct Arm {
    /// How long the arm was last frame.
    length: f32,
    /// Where the rig had the camera before any wall: what its damping continues from. Damping from
    /// the pulled-in position would have the rig believe the camera was there, and creep it back
    /// out at the damping's pace on top of the return's.
    free: Vec3,
    /// A return in progress: the length it started from, and the seconds it has run.
    returning: Option<(f32, f32)>,
}

impl Arms {
    /// Where the rig's damping continues from for `vcam`: the unobstructed position when a wall has
    /// been deciding, or `None` for a vcam no collision has touched.
    pub(crate) fn free_of(&self, vcam: Entity) -> Option<Vec3> {
        self.0.get(&vcam).map(|arm| arm.free)
    }

    /// What a gizmo draws: where the rig would have the camera, how long the arm is now, and
    /// whether a return is running. A wall's pull is invisible without it.
    pub fn held_of(&self, vcam: Entity) -> Option<(Vec3, f32, bool)> {
        self.0
            .get(&vcam)
            .map(|arm| (arm.free, arm.length, arm.returning.is_some()))
    }
}

/// The arm this frame, and the return clock to carry. Shorter than last frame is at once: a camera
/// that eased into a wall would show its inside. Longer is a tween that lasts **exactly**
/// `return_duration`, from wherever the arm was when the way cleared.
pub(crate) fn arm_length(
    previous: Option<(f32, Option<(f32, f32)>)>,
    clear: f32,
    collision: &CameraCollision,
    dt: f32,
) -> (f32, Option<(f32, f32)>) {
    let Some((was, _)) = previous else {
        return (clear, None);
    };
    // Going in or coming out are different speeds: a camera that eased into a wall would show the
    // inside of it, so `damping_when_occluded` is zero by default and the pull is immediate.
    let time = match clear <= was {
        true => collision.damping_when_occluded,
        false => collision.damping,
    };
    let length = was + (clear - was) * crate::virtual_camera::settled(dt, time);
    // Still carried, so a gizmo can say the arm is not where the rig would have it.
    let returning = match (length - clear).abs() > 1e-4 {
        true => Some((length, 0.0)),
        false => None,
    };
    (length, returning)
}

/// Where the camera of `vcam` goes this frame: `free`, or nearer the target along the same line
/// when something is in the way. The line keeps the framing — the camera still looks at the target
/// from the same side, only closer.
pub(crate) fn held(
    resources: &Resources,
    vcam: Entity,
    target: Vec3,
    target_entity: Option<Entity>,
    free: Vec3,
    arms: (&Arms, &mut Arms),
    dt: f32,
) -> Vec3 {
    let (carried, next) = arms;
    let Some(collision) = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<CameraCollision>())
        .and_then(|storage| storage.get(vcam))
        .copied()
        .filter(|collision| collision.enabled)
    else {
        return free;
    };
    let offset = free - target;
    let Some(direction) = offset.try_normalize() else {
        return free;
    };
    let full = offset.length();
    let clear = clear_length(
        resources,
        &collision,
        target,
        target_entity,
        direction,
        full,
    );
    let previous = carried.0.get(&vcam).map(|arm| (arm.length, arm.returning));
    let (length, returning) = arm_length(previous, clear, &collision, dt);
    let length = length.min(full);
    next.0.insert(
        vcam,
        Arm {
            length,
            free,
            returning,
        },
    );
    target + direction * length
}

/// How far from the target the camera can sit along `direction` before something stops it, up to
/// `full`. The sweep starts `min_distance` out, so nothing nearer the target than that counts.
#[cfg(feature = "physics")]
fn clear_length(
    resources: &Resources,
    collision: &CameraCollision,
    target: Vec3,
    target_entity: Option<Entity>,
    direction: Vec3,
    full: f32,
) -> f32 {
    use kooch_physics::backend::{CollisionShape, InteractionMask, QueryFilter, ShapeAt};

    let start = collision.min_distance.min(full);
    let Some(world) = resources.get::<kooch_physics::PhysicsWorld>() else {
        return full;
    };
    // The target's own body never stops the camera looking at it.
    let exclude = target_entity
        .and_then(|entity| {
            resources
                .get::<ComponentRegistry>()?
                .get_cpu::<kooch_physics::SolverBody>()?
                .get(entity)
                .copied()
        })
        .and_then(|body| world.handle(body.slot()));
    let filter = QueryFilter {
        exclude,
        groups: InteractionMask {
            memberships: u32::MAX,
            filter: collision.groups,
        },
        skip_sensors: true,
    };
    let sphere = CollisionShape::Sphere {
        radius: collision.radius.max(0.0),
    };
    let from = target + direction * start;
    match world
        .backend()
        .query_sweep(ShapeAt::new(&sphere, from), direction, full - start, filter)
    {
        Some(hit) => start + hit.t,
        None => full,
    }
}

/// Without a solver there is nothing to sweep against, and the arm is whole.
#[cfg(not(feature = "physics"))]
fn clear_length(
    _resources: &Resources,
    _collision: &CameraCollision,
    _target: Vec3,
    _target_entity: Option<Entity>,
    _direction: Vec3,
    full: f32,
) -> f32 {
    full
}

#[cfg(test)]
mod tests;

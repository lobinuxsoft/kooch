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
    /// Seconds to ease back out once the way clears. Pulling in is immediate: a camera that eased
    /// into a wall would show the inside of it.
    #[reflect(range = RETURN_RANGE)]
    pub return_time: f32,
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
            return_time: 0.35,
        }
    }
}

impl Component for CameraCollision {}

/// Per vcam, how long its arm was last frame: what easing back out measures from. Runtime state,
/// rebuilt from the vcams seen each frame.
#[derive(Debug, Clone, Default)]
pub struct ArmLengths(HashMap<Entity, f32>);

/// The arm this frame: the clear length at once when it is shorter than last frame's, eased towards
/// when it is longer. A time constant, so the feel does not change with frame rate.
pub(crate) fn arm_length(previous: Option<f32>, clear: f32, return_time: f32, dt: f32) -> f32 {
    match previous {
        Some(was) if clear > was && return_time > 0.0 => {
            was + (clear - was) * (1.0 - (-dt / return_time).exp())
        }
        _ => clear,
    }
}

/// Where the camera of `vcam` goes this frame: `wanted`, or nearer the target along the same line
/// when something is in the way. The line keeps the framing — the camera still looks at the target
/// from the same side, only closer.
pub(crate) fn held(
    resources: &Resources,
    vcam: Entity,
    target: Vec3,
    target_entity: Option<Entity>,
    wanted: Vec3,
    arms: (&ArmLengths, &mut ArmLengths),
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
        return wanted;
    };
    let offset = wanted - target;
    let Some(direction) = offset.try_normalize() else {
        return wanted;
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
    let length = arm_length(
        carried.0.get(&vcam).copied(),
        clear,
        collision.return_time,
        dt,
    )
    .min(full);
    next.0.insert(vcam, length);
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

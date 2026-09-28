//! Gizmos for the rest of the camera rig: the lead, the arm and the group (#1334).
//!
//! 🔴 These draw **state**, not settings. The Inspector already shows what was asked for; what
//! could not be seen was what the rig is doing with it — how far ahead the lead actually is, how
//! much of the arm a wall is holding, which point a group of targets resolves to. Three days of
//! this rig's bugs were found by playing because none of it was on screen.

use glam::{Vec3, Vec4};
use kooch_camera::CameraLookahead;
use kooch_camera::RigMemory;
use kooch_camera::occlusion::CameraCollision;
use kooch_camera::target::CameraTarget;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};

/// Ahead of the target: the same warm hue the vcam uses, lighter, since it is that rig's doing.
const LEAD: Vec3 = Vec3::new(1.0, 0.85, 0.45);
/// The cap the lead may not pass.
const CAP: Vec3 = Vec3::new(0.55, 0.45, 0.22);
/// A wall holding the arm in. Red enough to read as "something is in the way".
const HELD: Vec3 = Vec3::new(0.95, 0.4, 0.35);
/// The arm the rig would have had.
const FREE: Vec3 = Vec3::new(0.4, 0.55, 0.75);
/// A member of a target group.
const MEMBER: Vec3 = Vec3::new(0.55, 0.85, 0.95);
/// Where the group resolves to — what the camera actually follows.
const CENTRE: Vec3 = Vec3::new(0.95, 0.95, 0.6);

/// Segments in a drawn circle. Enough to read as one at arm's length.
const SEGMENTS: usize = 32;

/// Draws how far ahead of its target a vcam is holding the frame, and the ceiling on it.
#[derive(Default)]
pub(crate) struct LookaheadVisualizer;

impl Visualizer<CameraLookahead> for LookaheadVisualizer {
    fn draw(
        &self,
        _look: &CameraLookahead,
        _transform: &GlobalTransform,
        _gizmos: &mut Gizmos<'_>,
    ) {
        // The lead is state, so there is nothing to say without the resources.
    }

    fn draw_with(
        &self,
        look: &CameraLookahead,
        _transform: &GlobalTransform,
        entity: Entity,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        if !look.enabled {
            return;
        }
        let Some(target) = followed_point(resources, entity) else {
            return;
        };
        let offset = resources
            .get::<RigMemory>()
            .and_then(|memory| memory.leads.of(entity))
            .map(|lead| lead.offset())
            .unwrap_or(Vec3::ZERO);

        // The ceiling, drawn whether or not the lead has reached it: a cap you cannot see is a
        // number you cannot tune.
        let up = offset.try_normalize().unwrap_or(Vec3::Y);
        circle(gizmos, target, up, look.max_distance.max(0.0), CAP);

        // And the lead itself, from the target to where the frame is being held.
        if offset.length() > 1e-3 {
            gizmos.arrow(
                target,
                target + offset,
                Vec4::new(LEAD.x, LEAD.y, LEAD.z, 1.0),
            );
        }
    }
}

/// Draws the arm a wall is holding: where the rig would be, where it is, and the sweep between.
#[derive(Default)]
pub(crate) struct CameraCollisionVisualizer;

impl Visualizer<CameraCollision> for CameraCollisionVisualizer {
    fn draw(
        &self,
        _collision: &CameraCollision,
        _transform: &GlobalTransform,
        _gizmos: &mut Gizmos<'_>,
    ) {
    }

    fn draw_with(
        &self,
        collision: &CameraCollision,
        transform: &GlobalTransform,
        entity: Entity,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        if !collision.enabled {
            return;
        }
        let at = transform.matrix.to_scale_rotation_translation().2;
        let Some(target) = followed_point(resources, entity) else {
            return;
        };
        // The sweep the arm is tested along, and the radius it is tested with.
        gizmos.line(target, at, FREE);
        circle(
            gizmos,
            at,
            (at - target).normalize_or(Vec3::Y),
            collision.radius.max(0.0),
            FREE,
        );

        let Some((free, _, returning)) = resources
            .get::<RigMemory>()
            .and_then(|memory| memory.arms.held_of(entity))
        else {
            return;
        };
        // 🔴 Where the rig would have had the camera, against where the wall put it. Without this
        // a pull and a return look the same as the rig simply moving.
        if !free.abs_diff_eq(at, 1e-3) {
            gizmos.line(free, at, HELD);
            cross(gizmos, free, 0.15, FREE);
            cross(gizmos, at, 0.15, HELD);
        }
        if returning {
            circle(
                gizmos,
                at,
                (at - target).normalize_or(Vec3::Y),
                collision.min_distance.max(0.0),
                HELD,
            );
        }
    }
}

/// Draws a target's weight, and the point its group resolves to.
#[derive(Default)]
pub(crate) struct CameraTargetVisualizer;

impl Visualizer<CameraTarget> for CameraTargetVisualizer {
    fn draw(&self, target: &CameraTarget, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let at = transform.matrix.to_scale_rotation_translation().2;
        // Sized by weight, so which member pulls hardest is visible rather than read off a list.
        cross(gizmos, at, 0.2 + target.weight.max(0.0) * 0.3, MEMBER);
    }

    fn draw_with(
        &self,
        target: &CameraTarget,
        transform: &GlobalTransform,
        _entity: Entity,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        self.draw(target, transform, gizmos);
        // 🔴 The point the camera actually follows. With one member it is that member; with
        // several it is a place nothing stands, which is exactly why it has to be drawn.
        let Some(centre) = group_centre(resources, target.group) else {
            return;
        };
        let at = transform.matrix.to_scale_rotation_translation().2;
        if !centre.abs_diff_eq(at, 1e-3) {
            gizmos.line(at, centre, MEMBER);
            cross(gizmos, centre, 0.35, CENTRE);
        }
    }
}

/// The world point a vcam follows: its group's centre, or nothing when the group is empty.
fn followed_point(resources: &Resources, vcam: Entity) -> Option<Vec3> {
    let registry = resources.get::<kooch_ecs::component::ComponentRegistry>()?;
    let group = registry
        .get_cpu::<kooch_camera::VirtualCamera>()?
        .get(vcam)?
        .group;
    group_centre(resources, group)
}

/// Every member of `group`, weighted.
fn group_centre(resources: &Resources, group: u32) -> Option<Vec3> {
    let registry = resources.get::<kooch_ecs::component::ComponentRegistry>()?;
    let targets = registry.get_cpu::<CameraTarget>()?;
    let globals = registry.get_cpu::<GlobalTransform>()?;
    let members: Vec<(Vec3, f32)> = targets
        .iter()
        .filter(|(_, target)| target.group == group)
        .filter_map(|(entity, target)| {
            let at = globals
                .get(*entity)?
                .matrix
                .to_scale_rotation_translation()
                .2;
            Some((at, target.weight))
        })
        .collect();
    kooch_camera::target::weighted_centre(&members)
}

/// A circle of `radius` around `centre`, on the plane `axis` is normal to.
fn circle(gizmos: &mut Gizmos<'_>, centre: Vec3, axis: Vec3, radius: f32, colour: Vec3) {
    if radius <= 1e-3 {
        return;
    }
    let (a, b) = axis.normalize_or(Vec3::Y).any_orthonormal_pair();
    let point = |i: usize| {
        let t = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        centre + (a * t.cos() + b * t.sin()) * radius
    };
    for i in 0..SEGMENTS {
        gizmos.line(point(i), point(i + 1), colour);
    }
}

/// Three strokes through a point: a place, not a direction.
fn cross(gizmos: &mut Gizmos<'_>, at: Vec3, size: f32, colour: Vec3) {
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        gizmos.line(at - axis * size, at + axis * size, colour);
    }
}

#[cfg(test)]
mod tests;

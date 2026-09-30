//! Gizmo for [`VirtualCamera`].

use glam::Vec3;
use kooch_camera::RigMemory;
use kooch_camera::VirtualCamera;
use kooch_camera::virtual_camera::{UP_GRAVITY, UP_TARGET};
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};

use super::camera_rig::followed_point;

/// Warm, to separate it at a glance from the cool blues the real
/// cameras use — the difference between "this renders" and "this aims".
const VCAM_COLOR: Vec3 = Vec3::new(1.0, 0.75, 0.3);
/// A disabled vcam still draws, dimmed. Hiding it entirely makes a
/// switched-off framing indistinguishable from one that was deleted.
const DISABLED_COLOR: Vec3 = Vec3::new(0.45, 0.4, 0.32);
/// The axis a non-world up is aligned to, so it is visible that the
/// framing is not using +Y.
const UP_COLOR: Vec3 = Vec3::new(0.55, 0.9, 0.55);

/// Size of the marker, in world units. Fixed rather than derived from
/// anything: it marks a point, and scaling it with `distance` would make
/// a far-following camera draw a marker the size of the level.
const MARKER: f32 = 0.35;
/// How far the up axis sticks out. Longer than the marker so it reads as
/// a direction rather than part of the body.
const UP_LENGTH: f32 = 0.9;
/// Segments in the spring-arm orbit. Enough to read as a circle at the
/// distances a camera orbits from.
const ORBIT_SEGMENTS: usize = 48;
/// The pivot chain from the target to the camera. Cool against the vcam's warm body, because it is
/// the rig's doing rather than the marker's.
const RIG: Vec3 = Vec3::new(0.45, 0.8, 0.7);
/// A pivot on it.
const PIVOT: f32 = 0.08;
/// The three rings and the surface between them. Warmer than the shoulder chain, because it is the
/// shape the camera rides rather than a chain of points.
const RINGS: Vec3 = Vec3::new(0.8, 0.7, 0.45);
/// Points along the surface between the rings. Enough to read as a curve at arm's length.
const SURFACE: usize = 24;

/// Draws where a virtual camera is, which way it aims, and — in
/// third-person — the circle its spring arm swings around.
#[derive(Default)]
pub(crate) struct VirtualCameraVisualizer;

impl Visualizer<VirtualCamera> for VirtualCameraVisualizer {
    fn draw(&self, vcam: &VirtualCamera, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let colour = if vcam.enabled {
            VCAM_COLOR
        } else {
            DISABLED_COLOR
        };
        let to_world = |p: Vec3| transform.matrix.transform_point3(p);
        let origin = to_world(Vec3::ZERO);

        // A stubby pyramid down -Z: the direction the framing looks.
        let w = MARKER * 0.6;
        let mouth = [
            to_world(Vec3::new(w, w, -MARKER)),
            to_world(Vec3::new(-w, w, -MARKER)),
            to_world(Vec3::new(-w, -w, -MARKER)),
            to_world(Vec3::new(w, -w, -MARKER)),
        ];
        for i in 0..4 {
            gizmos.line(origin, mouth[i], colour);
            gizmos.line(mouth[i], mouth[(i + 1) % 4], colour);
        }

        // The up axis, when it is not simply +Y. Drawing it always would
        // add a line to every vcam to say "nothing unusual here".
        if vcam.up_mode == UP_GRAVITY || vcam.up_mode == UP_TARGET {
            let up = (to_world(Vec3::Y) - origin).normalize_or(Vec3::Y);
            gizmos.line(origin, origin + up * UP_LENGTH, UP_COLOR);
        }
    }

    /// The pivot chain the shoulder builds, drawn as Cinemachine draws it: root → shoulder → hand →
    /// camera, with a sphere on each.
    ///
    /// 🔴 State, not settings. A composer in the Aim turns the camera back to re-frame the target,
    /// which **cancels the shoulder's effect on screen** and leaves only the parallax — the offset is
    /// working and nothing says so. Measured: a `shoulder_offset.x` of 0.6 moves the character
    /// −0.117 of the screen under `Pan Tilt` and **0.000** under `Rotation Composer`. Cinemachine
    /// answers that by drawing the rig rather than warning about the combination, and so does this
    /// (#1379).
    fn draw_with(
        &self,
        vcam: &VirtualCamera,
        transform: &GlobalTransform,
        entity: Entity,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        self.draw(vcam, transform, gizmos);
        let Some(target) = followed_point(resources, entity) else {
            return;
        };
        // 🔴 What the rig used, where it has run. Where it has not — a stopped editor, which is
        // precisely when a shoulder is tuned — the rig's **own** first-step answers, not a second
        // opinion: `RigMemory` is not even inserted in the editor's process, and reading only it
        // meant this never drew at all (#1387).
        let (up, reference) = resources
            .get::<RigMemory>()
            .and_then(|memory| memory.horizons.used(entity))
            .unwrap_or_else(|| {
                let up = kooch_camera::up_for(vcam, resources, target, glam::Quat::IDENTITY);
                (up, kooch_camera::seed_reference(up))
            });
        // The surface an orbital rig rides, where it rides one: three circles and the spline joining
        // them. A shape you cannot see is a shape you cannot tune (#1379).
        if let Some(rings) = resources
            .get::<kooch_ecs::component::ComponentRegistry>()
            .and_then(|registry| kooch_camera::orbital_follow::of(registry, entity))
            // The component's presence is what says it rides one (#1397).
            .filter(|body| body.orbit_style == kooch_camera::ORBIT_THREE_RING)
        {
            let back = (transform.matrix.to_scale_rotation_translation().2 - target)
                .try_normalize()
                .map(|out| out - up * out.dot(up))
                .and_then(Vec3::try_normalize)
                .unwrap_or(reference);
            let mut previous = None;
            for step in 0..=SURFACE {
                let at = target + rings.at(step as f32 / SURFACE as f32, back, up);
                if let Some(from) = previous {
                    gizmos.line(from, at, RINGS);
                }
                previous = Some(at);
            }
            for t in [0.0, 0.5, 1.0] {
                let on = rings.at(t, back, up);
                circle(
                    gizmos,
                    target + up * on.dot(up),
                    up,
                    (on - up * on.dot(up)).length(),
                    RINGS,
                );
            }
        }
        let Some(body) = resources
            .get::<kooch_ecs::component::ComponentRegistry>()
            .and_then(|registry| kooch_camera::third_person_follow::of(registry, entity))
        else {
            return;
        };
        // The spring arm's orbit, from the body that knows how long it is: `camera_distance` and
        // `yaw` are otherwise two numbers with nothing to check them against.
        let camera = transform.matrix.to_scale_rotation_translation().2;
        let (root, shoulder, hand) = vcam.rig_positions(target, up, reference, body);
        if body.camera_distance > 1e-3 {
            // The hand the arm reaches back from: the last pivot, whatever the shoulder did.
            let centre = hand;
            circle(gizmos, centre, up, (camera - centre).length(), RIG * 0.8);
            gizmos.line(camera, centre, RIG * 0.8);
        }
        // Nothing authored collapses all three onto the target: a plain orbital rig draws the arm it
        // already had, not a chain of stubs on top of it.
        if root.abs_diff_eq(hand, 1e-4) {
            return;
        }
        gizmos.line(root, shoulder, RIG);
        gizmos.line(shoulder, hand, RIG);
        gizmos.line(hand, camera, RIG);
        for at in [root, shoulder, hand] {
            cross(gizmos, at, PIVOT, RIG);
        }
    }
}

/// A circle of `radius` around `centre`, on the plane `axis` is normal to.
fn circle(gizmos: &mut Gizmos<'_>, centre: Vec3, axis: Vec3, radius: f32, colour: Vec3) {
    if radius <= 1e-3 {
        return;
    }
    let start = match axis.cross(Vec3::X).try_normalize() {
        Some(side) => side,
        None => axis.cross(Vec3::Z).normalize(),
    };
    let side = axis.cross(start);
    let mut previous = centre + start * radius;
    for i in 1..=ORBIT_SEGMENTS {
        let a = i as f32 / ORBIT_SEGMENTS as f32 * std::f32::consts::TAU;
        let at = centre + (start * a.cos() + side * a.sin()) * radius;
        gizmos.line(previous, at, colour);
        previous = at;
    }
}

/// Three axis-aligned strokes through `at`. A sphere is what Cinemachine draws; three lines read the
/// same at a pivot's size and cost three lines.
fn cross(gizmos: &mut Gizmos<'_>, at: Vec3, size: f32, colour: Vec3) {
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        gizmos.line(at - axis * size, at + axis * size, colour);
    }
}

#[cfg(test)]
mod tests;

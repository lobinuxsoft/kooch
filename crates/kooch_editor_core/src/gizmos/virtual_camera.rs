//! Gizmo for [`VirtualCamera`].

use glam::Vec3;
use kooch_camera::RigMemory;
use kooch_camera::VirtualCamera;
use kooch_camera::virtual_camera::{FOLLOW_ORBITAL, FOLLOW_SHOULDER, UP_GRAVITY, UP_TARGET};
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

        // The spring arm's orbit. `distance` and `yaw` are otherwise two numbers with nothing to
        // check them against, and this is the circle the camera will swing along when yaw changes.
        let on_an_arm = vcam.follow == FOLLOW_ORBITAL || vcam.follow == FOLLOW_SHOULDER;
        if on_an_arm && vcam.camera_distance > 1e-3 {
            let forward = (to_world(-Vec3::Z) - origin).normalize_or(-Vec3::Z);
            let up = (to_world(Vec3::Y) - origin).normalize_or(Vec3::Y);
            let centre = origin + forward * vcam.camera_distance;

            // A basis on the orbit plane: perpendicular to up, through
            // the vcam. Using the vcam's own offset as the start angle
            // means the circle always passes through the marker.
            let radial = origin - centre;
            let radial_flat = radial - up * radial.dot(up);
            let Some(start) = radial_flat.try_normalize() else {
                return;
            };
            let radius = radial_flat.length();
            let side = up.cross(start);

            let mut prev = centre + start * radius;
            for i in 1..=ORBIT_SEGMENTS {
                let a = i as f32 / ORBIT_SEGMENTS as f32 * std::f32::consts::TAU;
                let p = centre + (start * a.cos() + side * a.sin()) * radius;
                gizmos.line(prev, p, colour * 0.55);
                prev = p;
            }
            // And the arm itself, so the radius is not just implied.
            gizmos.line(origin, centre, colour * 0.55);
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
        if vcam.follow != FOLLOW_SHOULDER {
            return;
        }
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
        let (root, shoulder, hand) = vcam.rig_positions(target, up, reference);
        // Nothing authored collapses all three onto the target: a plain orbital rig draws the arm it
        // already had, not a chain of stubs on top of it.
        if root.abs_diff_eq(hand, 1e-4) {
            return;
        }
        let camera = transform.matrix.to_scale_rotation_translation().2;
        gizmos.line(root, shoulder, RIG);
        gizmos.line(shoulder, hand, RIG);
        gizmos.line(hand, camera, RIG);
        for at in [root, shoulder, hand] {
            cross(gizmos, at, PIVOT, RIG);
        }
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

//! Gizmo for [`VirtualCamera`].
//!
//! A vcam has no mesh, no frustum of its own and nothing that renders,
//! so without a gizmo it is an empty entity in a list — you cannot see
//! where it is, which way it faces, or which of several is pointing
//! somewhere useful.
//!
//! It is deliberately *not* drawn like the real cameras. A
//! `PerspectiveCamera` gizmo shows its actual frustum, because it has a
//! fov and a near and far plane that mean something. A vcam has none of
//! that — it is a pose — so it gets a small fixed marker that reads as
//! "a viewpoint" without pretending to describe what will be seen
//! through it.

use glam::Vec3;
use kooch_camera::VirtualCamera;
use kooch_camera::virtual_camera::{FOLLOW_THIRD_PERSON, UP_GRAVITY, UP_TARGET};
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_gizmos::{Gizmos, Visualizer};

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

        // The spring arm's orbit. `distance` and `yaw` are otherwise two
        // numbers with nothing to check them against, and this is the
        // circle the camera will swing along when yaw changes.
        //
        // Centred on where the arm points *from*: the vcam sits one
        // `distance` away along its own forward axis, which is where the
        // target is whenever it is being looked at. When it is not, the
        // circle is still the right size and in the right plane — it is
        // the orbit, not the target.
        if vcam.follow == FOLLOW_THIRD_PERSON && vcam.distance > 1e-3 {
            let forward = (to_world(-Vec3::Z) - origin).normalize_or(-Vec3::Z);
            let up = (to_world(Vec3::Y) - origin).normalize_or(Vec3::Y);
            let centre = origin + forward * vcam.distance;

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
}

//! [`PlaneHandle`] — drag a small square at the corner of two axes to
//! translate in the plane formed by those axes.

use glam::{Vec3, Vec4};
use kooch_gizmos::Gizmos;

use crate::{Axis, DragInfo, Handle, HandleFrame, HandleMode, HandleState, Ray, TransformDelta};

/// Two-axis translate handle. Lives at the "corner" between two
/// cardinal axes and constrains drag to that plane.
///
/// For example, the X-Y plane handle (`axis_a = X, axis_b = Y`) lets
/// the user drag in the X-Y plane — Z stays fixed. Useful for
/// table-top-style positioning where the user wants to move along the
/// ground but not vertically.
///
/// Coloring follows the Unity convention: the plane handle is tinted
/// by the color of the **third** axis (the one perpendicular to the
/// plane, i.e. the constrained axis). X-Y plane → blue, X-Z → green,
/// Y-Z → red.
pub struct PlaneHandle {
    pub axis_a: Axis,
    pub axis_b: Axis,
    /// Distance from the origin along each axis to the near corner of
    /// the square. Default puts the square at 30% of the axis length.
    pub offset: f32,
    /// Side length of the square in world units.
    pub size: f32,
}

impl PlaneHandle {
    pub fn new(axis_a: Axis, axis_b: Axis) -> Self {
        Self {
            axis_a,
            axis_b,
            offset: 0.3,
            size: 0.3,
        }
    }

    /// Returns the third (perpendicular) axis whose color tints the handle.
    fn normal_axis(&self) -> Vec3 {
        match (self.axis_a, self.axis_b) {
            (Axis::X, Axis::Y) | (Axis::Y, Axis::X) => Axis::Z.base_color(),
            (Axis::X, Axis::Z) | (Axis::Z, Axis::X) => Axis::Y.base_color(),
            (Axis::Y, Axis::Z) | (Axis::Z, Axis::Y) => Axis::X.base_color(),
            _ => Vec3::ONE,
        }
    }

    /// Returns the four corners of the square in world space, plus the
    /// frame-rotated axis vectors and plane normal — all the geometry
    /// needed by `pick` / `drag` / `draw`.
    fn corners(&self, frame: HandleFrame) -> ([Vec3; 4], Vec3, Vec3, Vec3) {
        let a = frame.world_axis(self.axis_a);
        let b = frame.world_axis(self.axis_b);
        let normal = a.cross(b).normalize_or(Vec3::Y);
        let p0 = frame.origin + a * self.offset + b * self.offset;
        let p1 = p0 + a * self.size;
        let p2 = p1 + b * self.size;
        let p3 = p0 + b * self.size;
        ([p0, p1, p2, p3], a, b, normal)
    }
}

impl Handle for PlaneHandle {
    fn mode(&self) -> HandleMode {
        HandleMode::Translate
    }

    fn draw(&self, gizmos: &mut Gizmos<'_>, frame: HandleFrame, state: HandleState) {
        let base_color = self.normal_axis();
        let rgb = match state {
            HandleState::Idle => base_color,
            HandleState::Hover => bright(base_color),
            HandleState::Dragging => Vec3::new(1.0, 0.85, 0.2),
        };
        // Fill alpha 0.55 reads cleanly over the colorful SDF background;
        // the shader renders the perimeter with alpha 1.0 via per-vertex
        // `edge_uv`. Hover / drag feedback comes from the color shift,
        // not the alpha.
        let fill_color = Vec4::new(rgb.x, rgb.y, rgb.z, 0.55);
        let ([p0, p1, p2, p3], _, _, _) = self.corners(frame);
        gizmos.filled_quad(p0, p1, p2, p3, fill_color);
    }

    fn pick(&self, ray: Ray, frame: HandleFrame) -> Option<f32> {
        let (_, axis_a, axis_b, normal) = self.corners(frame);
        let t = ray_vs_plane(ray, frame.origin, normal)?;
        let hit = ray.at(t);
        let local = hit - frame.origin;
        let s_a = local.dot(axis_a);
        let s_b = local.dot(axis_b);
        let inside = s_a >= self.offset
            && s_a <= self.offset + self.size
            && s_b >= self.offset
            && s_b <= self.offset + self.size;
        if inside { Some(t) } else { None }
    }

    fn drag(&self, drag: DragInfo, frame: HandleFrame) -> TransformDelta {
        let (_, axis_a, axis_b, normal) = self.corners(frame);
        let start = ray_vs_plane(drag.start_ray, frame.origin, normal).map(|t| drag.start_ray.at(t));
        let last = ray_vs_plane(drag.last_ray, frame.origin, normal).map(|t| drag.last_ray.at(t));
        let current =
            ray_vs_plane(drag.current_ray, frame.origin, normal).map(|t| drag.current_ray.at(t));
        let translation = match (start, last, current) {
            (Some(s), Some(l), Some(c)) => {
                // Project all three onto the two plane axes (relative
                // to the start point) so snap math has a stable
                // anchor and toggling Ctrl mid-drag works smoothly.
                let last_a = (l - s).dot(axis_a);
                let last_b = (l - s).dot(axis_b);
                let now_a = (c - s).dot(axis_a);
                let now_b = (c - s).dot(axis_b);
                let (last_a, last_b, now_a, now_b) = if drag.modifiers.ctrl {
                    let step = drag.snap.translate;
                    (
                        snap_to(last_a, step),
                        snap_to(last_b, step),
                        snap_to(now_a, step),
                        snap_to(now_b, step),
                    )
                } else {
                    (last_a, last_b, now_a, now_b)
                };
                axis_a * (now_a - last_a) + axis_b * (now_b - last_b)
            }
            _ => Vec3::ZERO,
        };
        TransformDelta::Translation(translation)
    }
}

fn snap_to(value: f32, step: f32) -> f32 {
    if step.abs() < 1e-6 {
        return value;
    }
    (value / step).round() * step
}

// ---------------------------------------------------------------------------
// Math helpers
// ---------------------------------------------------------------------------

fn bright(c: Vec3) -> Vec3 {
    c.lerp(Vec3::ONE, 0.4)
}

/// Returns the distance along `ray` to the plane defined by
/// `(plane_origin, plane_normal)`. `None` when the ray is parallel to
/// the plane or hits behind the ray origin.
fn ray_vs_plane(ray: Ray, plane_origin: Vec3, plane_normal: Vec3) -> Option<f32> {
    let denom = ray.direction.dot(plane_normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (plane_origin - ray.origin).dot(plane_normal) / denom;
    if t < 0.0 { None } else { Some(t) }
}

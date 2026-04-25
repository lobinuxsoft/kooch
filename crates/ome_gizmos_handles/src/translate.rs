//! [`TranslateHandle`] — drag-an-axis-arrow to move the entity along it.

use glam::{Vec3, Vec4};
use ome_gizmos::Gizmos;

use crate::{Axis, DragInfo, Handle, HandleFrame, HandleMode, HandleState, Ray, TransformDelta};

/// Axis-aligned translate handle. One per cardinal axis (X / Y / Z).
///
/// - **Visual:** an arrow from `origin` to `origin + axis * length`,
///   with a 3D `+`-shaped arrowhead. Color brightens on hover and
///   while dragging.
/// - **Picking:** ray-vs-line-segment with a thickness threshold.
///   Tight enough to differentiate axes when arrows overlap on screen.
/// - **Drag math:** projects the cursor's world-space ray onto the
///   axis line, computes the difference between this frame's and
///   last frame's projection, returns the world-space delta along
///   that axis.
pub struct TranslateHandle {
    pub axis: Axis,
    pub length: f32,
    /// Distance threshold for picking a ray as "hitting" the arrow.
    /// In world units. Coarse-tuned for the default arrow length.
    pub pick_thickness: f32,
}

impl TranslateHandle {
    pub fn new(axis: Axis) -> Self {
        Self {
            axis,
            length: 1.0,
            pick_thickness: 0.08,
        }
    }
}

impl Handle for TranslateHandle {
    fn mode(&self) -> HandleMode {
        HandleMode::Translate
    }

    fn draw(&self, gizmos: &mut Gizmos<'_>, frame: HandleFrame, state: HandleState) {
        let rgb = match state {
            HandleState::Idle => self.axis.base_color(),
            HandleState::Hover => bright(self.axis.base_color()),
            HandleState::Dragging => Vec3::new(1.0, 0.85, 0.2), // selection-yellow while dragging
        };
        let dir = frame.world_axis(self.axis);
        let tip = frame.origin + dir * self.length;
        // Solid mesh arrow with full alpha — translates read better as
        // opaque shapes, unlike the translucent plane handles.
        gizmos.filled_arrow(frame.origin, tip, Vec4::new(rgb.x, rgb.y, rgb.z, 1.0));
    }

    fn pick(&self, ray: Ray, frame: HandleFrame) -> Option<f32> {
        let dir = frame.world_axis(self.axis);
        let p1 = frame.origin;
        let p2 = frame.origin + dir * self.length;
        ray_vs_segment(ray, p1, p2, self.pick_thickness)
    }

    fn drag(&self, drag: DragInfo, frame: HandleFrame) -> TransformDelta {
        let axis = frame.world_axis(self.axis);
        let last_s = project_ray_to_axis(drag.last_ray, frame.origin, axis);
        let current_s = project_ray_to_axis(drag.current_ray, frame.origin, axis);
        TransformDelta::Translation(axis * (current_s - last_s))
    }
}

// ---------------------------------------------------------------------------
// Math helpers
// ---------------------------------------------------------------------------

/// Brightens a color toward white for hover feedback.
fn bright(c: Vec3) -> Vec3 {
    c.lerp(Vec3::ONE, 0.4)
}

/// Projects a ray onto a line and returns the position `s` along the
/// line where `(line.origin + s * line.dir)` is closest to the ray.
///
/// Skew-line closest-approach math.
fn project_ray_to_axis(ray: Ray, axis_origin: Vec3, axis_dir: Vec3) -> f32 {
    let u = ray.origin - axis_origin;
    let b = ray.direction.dot(axis_dir);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-6 {
        // Ray parallel to axis: drag is undefined, return 0 to avoid jumps.
        return 0.0;
    }
    let d_ru = ray.direction.dot(u);
    let e_au = axis_dir.dot(u);
    (e_au - b * d_ru) / denom
}

/// Closest distance from `ray` to the line segment `[p1, p2]`. Returns
/// `Some(t_along_ray)` when the closest distance is below `threshold`
/// AND the closest point on the segment is within `[p1, p2]` AND the
/// ray hit is in front of the ray origin.
fn ray_vs_segment(ray: Ray, p1: Vec3, p2: Vec3, threshold: f32) -> Option<f32> {
    let segment = p2 - p1;
    let length = segment.length();
    if length < 1e-6 {
        return None;
    }
    let axis_dir = segment / length;

    let u = ray.origin - p1;
    let b = ray.direction.dot(axis_dir);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-6 {
        return None;
    }
    let d_ru = ray.direction.dot(u);
    let e_au = axis_dir.dot(u);
    let s = (e_au - b * d_ru) / denom;
    let t = b * s - d_ru;

    if t < 0.0 {
        return None;
    }
    if s < 0.0 || s > length {
        return None;
    }

    let closest_on_ray = ray.origin + ray.direction * t;
    let closest_on_segment = p1 + axis_dir * s;
    let dist = (closest_on_ray - closest_on_segment).length();

    if dist <= threshold { Some(t) } else { None }
}

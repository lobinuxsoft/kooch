//! [`TranslateHandle`] — drag-an-axis-arrow to move the entity along it.

use glam::{Vec3, Vec4};
use kooch_gizmos::Gizmos;

use crate::{Axis, DragInfo, Handle, HandleFrame, HandleMode, HandleState, Ray, TransformDelta};

/// Axis translate handle: an arrow along its axis, picked by ray-to-segment distance, dragged as
/// the change in the ray's projection onto the axis line.
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
        let start_s = project_ray_to_axis(drag.start_ray, frame.origin, axis);
        let last_s = project_ray_to_axis(drag.last_ray, frame.origin, axis);
        let current_s = project_ray_to_axis(drag.current_ray, frame.origin, axis);

        // Total distance from the click anchor, not a per-frame sum, so toggling snap mid-drag
        // rewinds at most one step.
        let total_last = last_s - start_s;
        let total_now = current_s - start_s;
        let (total_last, total_now) = if drag.modifiers.ctrl {
            let step = drag.snap.translate;
            (snap_to(total_last, step), snap_to(total_now, step))
        } else {
            (total_last, total_now)
        };
        TransformDelta::Translation(axis * (total_now - total_last))
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

/// Brightens a color toward white for hover feedback.
fn bright(c: Vec3) -> Vec3 {
    c.lerp(Vec3::ONE, 0.4)
}

/// Projects a ray onto a line: the `s` where `line.origin + s * line.dir` comes closest to the ray
/// (skew-line closest approach).
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

/// Ray distance to the segment `[p1, p2]`: `Some(t_along_ray)` when within `threshold`, inside the
/// segment, and in front of the ray origin.
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

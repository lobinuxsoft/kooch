//! [`RotateHandle`] — drag a torus around an axis to rotate the entity
//! around that axis.

use glam::{Quat, Vec3, Vec4};
use ome_gizmos::Gizmos;

use crate::{Axis, DragInfo, Handle, HandleFrame, HandleMode, HandleState, Ray, TransformDelta};

/// Axis-aligned rotate handle. One per cardinal axis (X / Y / Z).
///
/// - **Visual:** filled torus in the plane perpendicular to the axis,
///   coloured by the axis (X red / Y green / Z blue).
/// - **Picking:** ray-vs-plane intersection check, then verify the
///   hit point's distance from the entity origin is within the torus
///   tube (`major_radius ± minor_radius`).
/// - **Drag math:** project both rays onto the rotation plane, build
///   vectors from origin to each hit point, compute the signed angle
///   between them around the axis, return that as a quaternion.
pub struct RotateHandle {
    pub axis: Axis,
    pub major_radius: f32,
    pub minor_radius: f32,
}

impl RotateHandle {
    pub fn new(axis: Axis) -> Self {
        Self {
            axis,
            major_radius: 1.0,
            minor_radius: 0.04,
        }
    }
}

impl Handle for RotateHandle {
    fn mode(&self) -> HandleMode {
        HandleMode::Rotate
    }

    fn draw(&self, gizmos: &mut Gizmos<'_>, frame: HandleFrame, state: HandleState) {
        let rgb = match state {
            HandleState::Idle => self.axis.base_color(),
            HandleState::Hover => bright(self.axis.base_color()),
            HandleState::Dragging => Vec3::new(1.0, 0.85, 0.2),
        };
        let axis = frame.world_axis(self.axis);
        gizmos.filled_torus(
            frame.origin,
            axis,
            self.major_radius,
            self.minor_radius,
            Vec4::new(rgb.x, rgb.y, rgb.z, 1.0),
        );
    }

    fn pick(&self, ray: Ray, frame: HandleFrame) -> Option<f32> {
        let axis = frame.world_axis(self.axis);
        let t = ray_vs_plane(ray, frame.origin, axis)?;
        if t < 0.0 {
            return None;
        }
        let hit = ray.at(t);
        let radial = (hit - frame.origin).reject_from(axis);
        let radial_len = radial.length();
        let inner = self.major_radius - self.minor_radius;
        let outer = self.major_radius + self.minor_radius;
        if radial_len >= inner && radial_len <= outer {
            Some(t)
        } else {
            None
        }
    }

    fn drag(&self, drag: DragInfo, frame: HandleFrame) -> TransformDelta {
        let axis = frame.world_axis(self.axis);
        let start_hit = ray_vs_plane(drag.start_ray, frame.origin, axis)
            .map(|t| drag.start_ray.at(t));
        let last_hit = ray_vs_plane(drag.last_ray, frame.origin, axis)
            .map(|t| drag.last_ray.at(t));
        let curr_hit = ray_vs_plane(drag.current_ray, frame.origin, axis)
            .map(|t| drag.current_ray.at(t));
        let (start_hit, last_hit, curr_hit) = match (start_hit, last_hit, curr_hit) {
            (Some(s), Some(l), Some(c)) => (s, l, c),
            _ => return TransformDelta::Rotation(Quat::IDENTITY),
        };
        let v_start = (start_hit - frame.origin).reject_from(axis);
        let v_last = (last_hit - frame.origin).reject_from(axis);
        let v_curr = (curr_hit - frame.origin).reject_from(axis);
        if v_start.length_squared() < 1e-8
            || v_last.length_squared() < 1e-8
            || v_curr.length_squared() < 1e-8
        {
            return TransformDelta::Rotation(Quat::IDENTITY);
        }

        // Signed angle from the click anchor. Anchoring at `v_start`
        // makes snap toggling mid-drag stable.
        let total_last = signed_angle(v_start, v_last, axis);
        let total_now = signed_angle(v_start, v_curr, axis);
        let (total_last, total_now) = if drag.modifiers.ctrl {
            let step = drag.snap.rotate_deg.to_radians();
            (snap_to(total_last, step), snap_to(total_now, step))
        } else {
            (total_last, total_now)
        };

        let delta_angle = total_now - total_last;
        TransformDelta::Rotation(Quat::from_axis_angle(axis, delta_angle))
    }
}

/// Signed angle from `from` to `to` measured around `axis`.
fn signed_angle(from: Vec3, to: Vec3, axis: Vec3) -> f32 {
    let from_n = from.normalize();
    let to_n = to.normalize();
    let cos = from_n.dot(to_n).clamp(-1.0, 1.0);
    let unsigned = cos.acos();
    let sign = from_n.cross(to_n).dot(axis).signum();
    unsigned * sign
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

/// Returns `t` along `ray` where it hits the plane defined by
/// `(plane_origin, plane_normal)`. `None` if parallel or behind.
fn ray_vs_plane(ray: Ray, plane_origin: Vec3, plane_normal: Vec3) -> Option<f32> {
    let denom = ray.direction.dot(plane_normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (plane_origin - ray.origin).dot(plane_normal) / denom;
    if t < 0.0 { None } else { Some(t) }
}

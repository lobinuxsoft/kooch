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
        let last_hit = ray_vs_plane(drag.last_ray, frame.origin, axis)
            .map(|t| drag.last_ray.at(t));
        let curr_hit = ray_vs_plane(drag.current_ray, frame.origin, axis)
            .map(|t| drag.current_ray.at(t));
        let (last_hit, curr_hit) = match (last_hit, curr_hit) {
            (Some(l), Some(c)) => (l, c),
            _ => return TransformDelta::Rotation(Quat::IDENTITY),
        };
        let v_last = (last_hit - frame.origin).reject_from(axis);
        let v_curr = (curr_hit - frame.origin).reject_from(axis);
        if v_last.length_squared() < 1e-8 || v_curr.length_squared() < 1e-8 {
            return TransformDelta::Rotation(Quat::IDENTITY);
        }
        let v_last = v_last.normalize();
        let v_curr = v_curr.normalize();
        let cos_angle = v_last.dot(v_curr).clamp(-1.0, 1.0);
        let angle = cos_angle.acos();
        let sign = v_last.cross(v_curr).dot(axis).signum();
        TransformDelta::Rotation(Quat::from_axis_angle(axis, angle * sign))
    }
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

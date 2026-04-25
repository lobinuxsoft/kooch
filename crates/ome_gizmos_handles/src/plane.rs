//! [`PlaneHandle`] — drag a small square at the corner of two axes to
//! translate in the plane formed by those axes.

use glam::Vec3;
use ome_gizmos::Gizmos;

use crate::{Axis, DragInfo, Handle, HandleState, Ray};

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

    /// Plane normal (cross product of the two axes).
    fn normal(&self) -> Vec3 {
        self.axis_a.vec().cross(self.axis_b.vec()).normalize()
    }

    /// Returns the four corners of the square in world space.
    fn corners(&self, origin: Vec3) -> [Vec3; 4] {
        let a = self.axis_a.vec();
        let b = self.axis_b.vec();
        let p0 = origin + a * self.offset + b * self.offset;
        let p1 = p0 + a * self.size;
        let p2 = p1 + b * self.size;
        let p3 = p0 + b * self.size;
        [p0, p1, p2, p3]
    }
}

impl Handle for PlaneHandle {
    fn draw(&self, gizmos: &mut Gizmos<'_>, origin: Vec3, state: HandleState) {
        let base_color = self.normal_axis();
        let color = match state {
            HandleState::Idle => base_color,
            HandleState::Hover => bright(base_color),
            HandleState::Dragging => Vec3::new(1.0, 0.85, 0.2),
        };
        let [p0, p1, p2, p3] = self.corners(origin);
        gizmos.line(p0, p1, color);
        gizmos.line(p1, p2, color);
        gizmos.line(p2, p3, color);
        gizmos.line(p3, p0, color);
        // Diagonal cross to make hover target obvious until filled
        // translucent quads arrive in sub-phase 3b.
        gizmos.line(p0, p2, color);
        gizmos.line(p1, p3, color);
    }

    fn pick(&self, ray: Ray, origin: Vec3) -> Option<f32> {
        let normal = self.normal();
        let plane_origin = origin;
        let t = ray_vs_plane(ray, plane_origin, normal)?;
        let hit = ray.at(t);
        // Project hit onto the two axes; bail if outside the square.
        let local = hit - origin;
        let s_a = local.dot(self.axis_a.vec());
        let s_b = local.dot(self.axis_b.vec());
        let inside = s_a >= self.offset
            && s_a <= self.offset + self.size
            && s_b >= self.offset
            && s_b <= self.offset + self.size;
        if inside { Some(t) } else { None }
    }

    fn drag(&self, drag: DragInfo, origin: Vec3) -> Vec3 {
        let normal = self.normal();
        let last = ray_vs_plane(drag.last_ray, origin, normal).map(|t| drag.last_ray.at(t));
        let current =
            ray_vs_plane(drag.current_ray, origin, normal).map(|t| drag.current_ray.at(t));
        match (last, current) {
            (Some(l), Some(c)) => {
                let delta = c - l;
                // Constrain to the two plane axes (the projection onto the
                // plane already does this, but the floating-point residue
                // could leak into the third axis — explicit project keeps
                // things clean).
                let a = self.axis_a.vec();
                let b = self.axis_b.vec();
                a * delta.dot(a) + b * delta.dot(b)
            }
            _ => Vec3::ZERO,
        }
    }
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

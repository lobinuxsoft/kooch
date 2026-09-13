//! Wireframe outlines for the curved shapes a collider can be.
//! Outlines, not solid meshes: a collider usually sits inside its visual mesh. Each takes a `Mat3`
//! basis so the outline turns with the entity.

use glam::{Mat3, Vec3};

use crate::Gizmos;

/// Target chord error for a circle, in world units. Constant error, not a fixed segment count, so
/// small and large circles both look round.
const CHORD_ERROR: f32 = 0.01;

/// Fewest segments any circle is drawn with, so tiny circles do not read as polygons.
pub const MIN_CIRCLE_SEGMENTS: u32 = 32;

/// Most segments any circle is drawn with, so planet-scale radii do not flood the line batch.
pub const MAX_CIRCLE_SEGMENTS: u32 = 96;

/// Segments for a circle of `radius` at constant chord error: `n ≈ π·sqrt(r / 2e)`, so the count
/// grows with the square root of the radius.
pub fn segments_for(radius: f32) -> u32 {
    let radius = radius.abs().max(f32::EPSILON);
    let ideal = std::f32::consts::PI * (radius / (2.0 * CHORD_ERROR)).sqrt();
    (ideal.ceil() as u32).clamp(MIN_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS)
}

impl Gizmos<'_> {
    /// The twelve edges of an oriented box, turning with a rotated entity.
    pub fn wire_obb(&mut self, centre: Vec3, basis: Mat3, half_extents: Vec3, color: Vec3) {
        let (x, y, z) = (
            basis.x_axis * half_extents.x,
            basis.y_axis * half_extents.y,
            basis.z_axis * half_extents.z,
        );
        // The eight corners, indexed so bit 0 is ±x, bit 1 is ±y, bit 2 ±z.
        let corner = |i: usize| {
            centre
                + if i & 1 == 0 { -x } else { x }
                + if i & 2 == 0 { -y } else { y }
                + if i & 4 == 0 { -z } else { z }
        };
        // An edge joins corners differing in exactly one bit; taking only
        // the pairs where the lower index is smaller draws each once.
        for a in 0..8usize {
            for bit in [1, 2, 4] {
                let b = a ^ bit;
                if a < b {
                    self.line(corner(a), corner(b), color);
                }
            }
        }
    }

    /// One circle of `radius` in the plane spanned by `u` and `v` — the building block of every
    /// rounded outline.
    pub fn wire_circle(&mut self, centre: Vec3, u: Vec3, v: Vec3, radius: f32, color: Vec3) {
        self.wire_arc(centre, u, v, radius, 0.0, std::f32::consts::TAU, color);
    }

    /// Part of a circle, from `start` to `end` radians measured from `u`
    /// towards `v`.
    pub fn wire_arc(
        &mut self,
        centre: Vec3,
        u: Vec3,
        v: Vec3,
        radius: f32,
        start: f32,
        end: f32,
        color: Vec3,
    ) {
        let span = end - start;
        // Density from the radius, then scaled by how much of the circle
        // this arc covers — so a quarter arc is not drawn as coarsely as a
        // full circle of the same radius.
        let full = segments_for(radius) as f32;
        let segments = ((full * (span.abs() / std::f32::consts::TAU)).ceil() as u32).max(2);
        let point = |t: f32| {
            let angle = start + span * t;
            centre + (u * angle.cos() + v * angle.sin()) * radius
        };
        let mut previous = point(0.0);
        for i in 1..=segments {
            let next = point(i as f32 / segments as f32);
            self.line(previous, next, color);
            previous = next;
        }
    }

    /// Sphere outline: three great circles, which read as a sphere from any angle without hiding
    /// what is behind.
    pub fn wire_sphere(&mut self, centre: Vec3, basis: Mat3, radius: f32, color: Vec3) {
        let (x, y, z) = (basis.x_axis, basis.y_axis, basis.z_axis);
        self.wire_circle(centre, x, y, radius, color);
        self.wire_circle(centre, y, z, radius, color);
        self.wire_circle(centre, z, x, radius, color);
    }

    /// Capsule outline along the basis' Y axis. `half_height` excludes the caps, as in
    /// `CollisionShape` and rapier's `capsule_y`.
    pub fn wire_capsule(
        &mut self,
        centre: Vec3,
        basis: Mat3,
        radius: f32,
        half_height: f32,
        color: Vec3,
    ) {
        let (x, y, z) = (basis.x_axis, basis.y_axis, basis.z_axis);
        let top = centre + y * half_height;
        let bottom = centre - y * half_height;

        // The two rings where the caps meet the body.
        self.wire_circle(top, x, z, radius, color);
        self.wire_circle(bottom, x, z, radius, color);

        // Four silhouette lines down the body.
        for dir in [x, -x, z, -z] {
            self.line(top + dir * radius, bottom + dir * radius, color);
        }

        // Hemispherical caps: two half-circles each, in the vertical
        // planes, so the dome is visible from any side.
        let half = std::f32::consts::PI;
        for plane in [(x, y), (z, y)] {
            self.wire_arc(top, plane.0, plane.1, radius, 0.0, half, color);
            self.wire_arc(bottom, plane.0, -plane.1, radius, 0.0, half, color);
        }
    }

    /// Cylinder outline along the basis' Y axis. Height is
    /// `2 * half_height`.
    pub fn wire_cylinder(
        &mut self,
        centre: Vec3,
        basis: Mat3,
        radius: f32,
        half_height: f32,
        color: Vec3,
    ) {
        let (x, y, z) = (basis.x_axis, basis.y_axis, basis.z_axis);
        let top = centre + y * half_height;
        let bottom = centre - y * half_height;
        self.wire_circle(top, x, z, radius, color);
        self.wire_circle(bottom, x, z, radius, color);
        for dir in [x, -x, z, -z] {
            self.line(top + dir * radius, bottom + dir * radius, color);
        }
    }

    /// Cone outline along the basis' Y axis: base at `-half_height`,
    /// apex at `+half_height`.
    pub fn wire_cone(
        &mut self,
        centre: Vec3,
        basis: Mat3,
        radius: f32,
        half_height: f32,
        color: Vec3,
    ) {
        let (x, y, z) = (basis.x_axis, basis.y_axis, basis.z_axis);
        let base = centre - y * half_height;
        let apex = centre + y * half_height;
        self.wire_circle(base, x, z, radius, color);
        for dir in [x, -x, z, -z] {
            self.line(base + dir * radius, apex, color);
        }
    }

    /// Half-space outline: a bounded grid patch on the plane plus an arrow along `normal`, since
    /// which side is solid is what matters.
    pub fn wire_halfspace(&mut self, origin: Vec3, normal: Vec3, extent: f32, color: Vec3) {
        let normal = normal.normalize_or(Vec3::Y);
        let u = normal.any_orthonormal_vector();
        let v = normal.cross(u);
        let steps = 4;
        for i in 0..=steps {
            let t = (i as f32 / steps as f32 * 2.0 - 1.0) * extent;
            self.line(
                origin + u * t - v * extent,
                origin + u * t + v * extent,
                color,
            );
            self.line(
                origin + v * t - u * extent,
                origin + v * t + u * extent,
                color,
            );
        }
        self.line(origin, origin + normal * extent * 0.5, color);
    }

    /// Every edge of a triangle soup, deduplicated — a closed trimesh shares each edge between two
    /// triangles.
    pub fn wire_triangles(&mut self, vertices: &[Vec3], indices: &[[u32; 3]], color: Vec3) {
        let mut seen = std::collections::HashSet::with_capacity(indices.len() * 3);
        for tri in indices {
            for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                // Order-independent key so an edge shared by two
                // triangles with opposite winding is still one edge.
                let key = (a.min(b), a.max(b));
                if !seen.insert(key) {
                    continue;
                }
                if let (Some(&start), Some(&end)) =
                    (vertices.get(a as usize), vertices.get(b as usize))
                {
                    self.line(start, end, color);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

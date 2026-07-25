//! Wireframe outlines for the curved shapes a collider can be.
//!
//! # Why wireframe and not a translucent mesh
//!
//! A collider usually sits *inside* the visual mesh it belongs to, so a
//! solid representation is hidden exactly when you need to look at it.
//! Unity, Unreal and Godot all draw an outline instead: you see the
//! capsule around the character, not a green blob where the character
//! used to be.
//!
//! # Basis, not axis flags
//!
//! Every function takes a `Mat3` basis so the outline follows the
//! entity's rotation, the same way [`Gizmos::filled_obb`] already does.
//! Drawing a rotated body's collider axis-aligned would be worse than
//! drawing nothing: it would look like the collider had not rotated.
//!
//! [`Gizmos::filled_obb`]: crate::Gizmos::filled_obb

use glam::{Mat3, Vec3};

use crate::Gizmos;

/// Segments per full circle. Sixteen reads as round at editor distances
/// without flooding the line batch — a selection of a dozen bodies is
/// otherwise thousands of segments for shapes nobody is inspecting.
pub const CIRCLE_SEGMENTS: u32 = 16;

impl Gizmos<'_> {
    /// The twelve edges of an oriented box.
    ///
    /// [`Gizmos::aabb`] draws an axis-aligned one; a collider on a rotated
    /// entity needs its box to turn with it, and
    /// [`Gizmos::filled_obb`] draws a solid one you cannot see through.
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

    /// One circle of `radius` in the plane spanned by `u` and `v`.
    ///
    /// The building block for every rounded outline below: a sphere is
    /// three of these, a capsule two half-circles plus two, a cylinder
    /// two plus its silhouette.
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
        // Keep the segment density constant so a quarter arc is not
        // drawn as coarsely as a full circle.
        let segments =
            ((CIRCLE_SEGMENTS as f32 * (span.abs() / std::f32::consts::TAU)).ceil() as u32).max(2);
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

    /// Sphere outline: three great circles, one per basis plane.
    ///
    /// Three rather than a full latitude/longitude grid — that is the
    /// representation every engine settled on, because it reads as a
    /// sphere from any angle without obscuring what is behind it.
    pub fn wire_sphere(&mut self, centre: Vec3, basis: Mat3, radius: f32, color: Vec3) {
        let (x, y, z) = (basis.x_axis, basis.y_axis, basis.z_axis);
        self.wire_circle(centre, x, y, radius, color);
        self.wire_circle(centre, y, z, radius, color);
        self.wire_circle(centre, z, x, radius, color);
    }

    /// Capsule outline along the basis' Y axis.
    ///
    /// `half_height` excludes the caps, matching both `CollisionShape` and
    /// rapier's `capsule_y`, so total height is
    /// `2 * (half_height + radius)`.
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

    /// Half-space outline: a bounded grid patch on the plane through
    /// `origin` with the given `normal`, plus an arrow along it.
    ///
    /// An infinite plane cannot be drawn, so it is suggested: the patch
    /// says where the surface is and the arrow says which side is solid.
    /// Without the arrow a half-space is indistinguishable from a
    /// double-sided plane, and which side is solid is the only thing
    /// about it that matters.
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

    /// Every edge of a triangle soup, for the mesh-derived colliders.
    ///
    /// Deduplicates shared edges: a closed trimesh shares each edge
    /// between two triangles, so drawing them per-triangle doubles the
    /// line count for an identical picture.
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

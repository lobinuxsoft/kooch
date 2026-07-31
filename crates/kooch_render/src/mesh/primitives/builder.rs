//! [`MeshBuilder`] — the shared accumulator every primitive writes into.
//!
//! Keeps the winding and vertex-layout rules in one place instead of
//! repeating them in six generators. glTF's conventions are the engine's:
//! right-handed, Y up, counter-clockwise front faces. A primitive that
//! gets that wrong is inside-out, and an inside-out mesh reads as a
//! backface-culling bug rather than a generator bug.

use glam::{Vec2, Vec3};

use crate::mesh::{Mesh, MeshVertex};

/// Accumulates vertices and triangles for one primitive.
#[derive(Default)]
pub(super) struct MeshBuilder {
    vertices: Vec<MeshVertex>,
    indices: Vec<u32>,
}

impl MeshBuilder {
    /// Reserves room for a known vertex and triangle count.
    pub(super) fn with_capacity(vertices: usize, triangles: usize) -> Self {
        Self {
            vertices: Vec::with_capacity(vertices),
            indices: Vec::with_capacity(triangles * 3),
        }
    }

    /// Pushes a vertex and returns its index.
    ///
    /// `normal` is normalised here rather than at every call site — a
    /// generator computing an analytic normal should not also have to
    /// remember that the GPU expects unit length.
    pub(super) fn vertex(&mut self, position: Vec3, normal: Vec3, uv: Vec2) -> u32 {
        let index = self.vertices.len() as u32;
        self.vertices.push(MeshVertex {
            position: position.to_array(),
            normal: normal.normalize_or_zero().to_array(),
            uv: uv.to_array(),
        });
        index
    }

    /// Index the next pushed vertex will get. Generators that stitch a
    /// grid need the base index of the run they are about to write.
    pub(super) fn vertices_len(&self) -> u32 {
        self.vertices.len() as u32
    }

    /// Adds a triangle. `a → b → c` counter-clockwise seen from outside.
    pub(super) fn triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    /// Adds a quad as two triangles, `a → b → c → d` counter-clockwise.
    pub(super) fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.triangle(a, b, c);
        self.triangle(a, c, d);
    }

    /// Finishes the mesh, computing the AABB from the positions.
    pub(super) fn build(self) -> Mesh {
        Mesh::from_arrays(self.vertices, self.indices)
    }
}

/// A ring of `sectors` positions on a circle of `radius` at height `y`.
///
/// Shared by the sphere, capsule, cylinder and cone. Returns
/// `sectors + 1` entries: the seam is duplicated so the last vertex can
/// carry `u = 1.0` instead of wrapping to `0.0`, which would smear the
/// whole texture across the final column.
pub(super) fn ring(radius: f32, y: f32, sectors: u32) -> Vec<Vec3> {
    (0..=sectors)
        .map(|s| {
            let theta = s as f32 / sectors as f32 * std::f32::consts::TAU;
            Vec3::new(radius * theta.cos(), y, radius * theta.sin())
        })
        .collect()
}

/// Minimum segment count for anything round.
///
/// Below three there is no surface — two sectors give a degenerate sliver
/// with no volume, which a convex hull or an inertia tensor then divides
/// by. Clamped rather than rejected: a value being typed into the
/// Inspector passes through 0 and 1 on its way to the intended number.
pub(super) const MIN_SECTORS: u32 = 3;

/// Minimum ring count for shapes stacked along their axis.
pub(super) const MIN_RINGS: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_closes_the_seam_with_a_duplicate() {
        let r = ring(1.0, 0.0, 4);
        assert_eq!(r.len(), 5, "the seam vertex is not duplicated");
        assert!(
            r[0].abs_diff_eq(r[4], 1e-5),
            "first and last ring positions differ: {:?} vs {:?}",
            r[0],
            r[4]
        );
    }

    #[test]
    fn ring_sits_on_the_circle_at_the_requested_height() {
        for p in ring(2.0, 3.0, 8) {
            assert!((Vec2::new(p.x, p.z).length() - 2.0).abs() < 1e-5);
            assert_eq!(p.y, 3.0);
        }
    }

    #[test]
    fn builder_normalises_normals() {
        let mut b = MeshBuilder::default();
        b.vertex(Vec3::ZERO, Vec3::new(0.0, 5.0, 0.0), Vec2::ZERO);
        let mesh = b.build();
        assert_eq!(mesh.vertices[0].normal, [0.0, 1.0, 0.0]);
    }
}

//! Cube and quad — the two primitives with no curvature.
//!
//! Both duplicate vertices per face rather than sharing corners. A cube
//! with eight shared vertices has to average three perpendicular normals
//! at every corner, which rounds the lighting off the edges and makes a
//! box look like a beanbag. Twenty-four vertices buy flat faces.

use glam::{Vec2, Vec3};

use crate::mesh::Mesh;

use super::builder::MeshBuilder;

/// Axis-aligned box centred on the origin.
pub(super) fn cube(half_extents: Vec3) -> Mesh {
    let h = half_extents.max(Vec3::splat(super::MIN_EXTENT));
    let mut b = MeshBuilder::with_capacity(24, 12);

    // (normal, tangent, bitangent) per face. The quad is built as
    // -t-bt, +t-bt, +t+bt, -t+bt, which winds counter-clockwise when
    // viewed from along +normal for a right-handed (t, bt, n) basis.
    let faces = [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ];

    for (normal, tangent, bitangent) in faces {
        let centre = normal * h;
        let t = tangent * h;
        let bt = bitangent * h;
        let a = b.vertex(centre - t - bt, normal, Vec2::new(0.0, 1.0));
        let c = b.vertex(centre + t - bt, normal, Vec2::new(1.0, 1.0));
        let d = b.vertex(centre + t + bt, normal, Vec2::new(1.0, 0.0));
        let e = b.vertex(centre - t + bt, normal, Vec2::new(0.0, 0.0));
        b.quad(a, c, d, e);
    }

    b.build()
}

/// Single-sided plane on XZ, facing +Y, centred on the origin.
///
/// Y up and facing up because its job is being a floor.
pub(super) fn quad(half_extents: Vec2) -> Mesh {
    let h = half_extents.max(Vec2::splat(super::MIN_EXTENT));
    let mut b = MeshBuilder::with_capacity(4, 2);

    // Seen from +Y looking down, -Z is "up" on screen, so winding
    // -x-z → +x-z → +x+z → -x+z is counter-clockwise from above.
    let a = b.vertex(Vec3::new(-h.x, 0.0, -h.y), Vec3::Y, Vec2::new(0.0, 0.0));
    let c = b.vertex(Vec3::new(h.x, 0.0, -h.y), Vec3::Y, Vec2::new(1.0, 0.0));
    let d = b.vertex(Vec3::new(h.x, 0.0, h.y), Vec3::Y, Vec2::new(1.0, 1.0));
    let e = b.vertex(Vec3::new(-h.x, 0.0, h.y), Vec3::Y, Vec2::new(0.0, 1.0));
    b.quad(a, e, d, c);

    b.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitives::tests::{
        assert_outward_facing, assert_unit_normals, assert_uvs_in_unit_range,
    };

    #[test]
    fn cube_has_flat_faces_and_encloses_its_extents() {
        let mesh = cube(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(
            mesh.vertex_count(),
            24,
            "corners were shared, faces will smear"
        );
        assert_eq!(mesh.index_count(), 36);
        assert_eq!(mesh.aabb.min, Vec3::new(-1.0, -2.0, -3.0));
        assert_eq!(mesh.aabb.max, Vec3::new(1.0, 2.0, 3.0));
        assert_unit_normals(&mesh);
        assert_uvs_in_unit_range(&mesh);
        assert_outward_facing(&mesh);
    }

    /// Six distinct face normals, one per side, all axis-aligned.
    #[test]
    fn cube_normals_are_one_per_face() {
        let mesh = cube(Vec3::ONE);
        let mut seen: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.normal).collect();
        seen.sort_by(|a, b| a.partial_cmp(b).unwrap());
        seen.dedup();
        assert_eq!(seen.len(), 6, "expected exactly six face normals: {seen:?}");
    }

    #[test]
    fn quad_faces_up() {
        let mesh = quad(Vec2::splat(5.0));
        assert_eq!(mesh.vertex_count(), 4);
        assert_eq!(mesh.index_count(), 6);
        for v in &mesh.vertices {
            assert_eq!(v.normal, [0.0, 1.0, 0.0]);
            assert_eq!(v.position[1], 0.0);
        }
        assert_uvs_in_unit_range(&mesh);
    }

    /// Its winding has to agree with its normal, or a floor is invisible
    /// from above and solid from below.
    #[test]
    fn quad_winding_agrees_with_its_normal() {
        let mesh = quad(Vec2::ONE);
        let p = |i: u32| Vec3::from_array(mesh.vertices[i as usize].position);
        for tri in mesh.indices.chunks(3) {
            let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
            let geometric = (b - a).cross(c - a);
            assert!(
                geometric.dot(Vec3::Y) > 0.0,
                "triangle winds clockwise from above: {geometric:?}"
            );
        }
    }

    #[test]
    fn degenerate_extents_are_clamped() {
        let mesh = cube(Vec3::ZERO);
        assert!(
            mesh.aabb.max.min_element() > 0.0,
            "a zero cube has no volume"
        );
        let mesh = quad(Vec2::ZERO);
        assert!(mesh.aabb.max.x > 0.0);
    }
}

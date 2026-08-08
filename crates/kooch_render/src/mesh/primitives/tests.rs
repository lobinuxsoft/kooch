use super::*;

/// Every normal is unit length. A zero or over-long normal darkens or
/// blows out the shading, and looks like a lighting bug.
pub(crate) fn assert_unit_normals(mesh: &Mesh) {
    for (i, v) in mesh.vertices.iter().enumerate() {
        let len = Vec3::from_array(v.normal).length();
        assert!(
            (len - 1.0).abs() < 1e-3,
            "vertex {i} normal is not unit length: {len}"
        );
    }
}

/// UVs stay inside `[0, 1]`. Outside it a clamped sampler smears the
/// edge texel across the whole face.
pub(crate) fn assert_uvs_in_unit_range(mesh: &Mesh) {
    for (i, v) in mesh.vertices.iter().enumerate() {
        assert!(
            (-1e-4..=1.0 + 1e-4).contains(&v.uv[0]) && (-1e-4..=1.0 + 1e-4).contains(&v.uv[1]),
            "vertex {i} uv out of range: {:?}",
            v.uv
        );
    }
}

/// Winding agrees with the vertex normals: the geometric normal of
/// every non-degenerate triangle points the same way as its corners'.
///
/// This is the assertion that catches an inside-out primitive, which
/// otherwise only shows up as an invisible mesh once backface culling
/// is on.
pub(crate) fn assert_outward_facing(mesh: &Mesh) {
    let position = |i: u32| Vec3::from_array(mesh.vertices[i as usize].position);
    let normal = |i: u32| Vec3::from_array(mesh.vertices[i as usize].normal);
    for (t, tri) in mesh.indices.chunks(3).enumerate() {
        let (a, b, c) = (tri[0], tri[1], tri[2]);
        let geometric = (position(b) - position(a)).cross(position(c) - position(a));
        // Degenerate triangles (the poles of a UV sphere) have no
        // geometric normal to compare against.
        if geometric.length() < 1e-6 {
            continue;
        }
        let averaged = normal(a) + normal(b) + normal(c);
        assert!(
            geometric.normalize().dot(averaged.normalize_or_zero()) > 0.0,
            "triangle {t} winds inward: geometric {geometric:?} vs shading {averaged:?}"
        );
    }
}

#[test]
fn every_canonical_primitive_builds_a_usable_mesh() {
    for (name, primitive) in Primitive::CANONICAL {
        let mesh = primitive.build();
        assert!(mesh.vertex_count() >= 3, "{name} has no vertices");
        assert_eq!(mesh.index_count() % 3, 0, "{name} has a partial triangle");
        assert!(!mesh.aabb.is_empty(), "{name} has empty bounds");
        assert_unit_normals(&mesh);
        assert_uvs_in_unit_range(&mesh);
        assert_outward_facing(&mesh);
    }
}

/// Indices address real vertices. An out-of-range index is a GPU
/// crash or garbage geometry, not a visual glitch.
#[test]
fn every_index_is_in_range() {
    for (name, primitive) in Primitive::CANONICAL {
        let mesh = primitive.build();
        let count = mesh.vertex_count();
        for &i in &mesh.indices {
            assert!(i < count, "{name} index {i} exceeds {count} vertices");
        }
    }
}

/// Building the same recipe twice gives the same mesh — the baked
/// assets have to be reproducible, or their GUIDs churn.
#[test]
fn generation_is_deterministic() {
    for (name, primitive) in Primitive::CANONICAL {
        let a = primitive.build();
        let b = primitive.build();
        assert_eq!(a.indices, b.indices, "{name} indices differ between runs");
        let positions =
            |m: &Mesh| -> Vec<[f32; 3]> { m.vertices.iter().map(|v| v.position).collect() };
        assert_eq!(positions(&a), positions(&b), "{name} positions differ");
    }
}

use super::*;

fn vertex(p: [f32; 3]) -> MeshVertex {
    MeshVertex {
        position: p,
        normal: [0.0, 1.0, 0.0],
        uv: [0.0, 0.0],
    }
}

#[test]
fn empty_mesh_has_zero_counts() {
    let m = Mesh::empty();
    assert_eq!(m.vertex_count(), 0);
    assert_eq!(m.index_count(), 0);
    assert!(m.aabb.is_empty());
}

#[test]
fn from_arrays_computes_aabb_from_positions() {
    let verts = vec![
        vertex([0.0, 0.0, 0.0]),
        vertex([2.0, 4.0, -1.0]),
        vertex([-3.0, 1.0, 5.0]),
    ];
    let idx = vec![0, 1, 2];
    let mesh = Mesh::from_arrays(verts, idx);

    assert_eq!(mesh.vertex_count(), 3);
    assert_eq!(mesh.index_count(), 3);
    assert_eq!(mesh.aabb.min, Vec3::new(-3.0, 0.0, -1.0));
    assert_eq!(mesh.aabb.max, Vec3::new(2.0, 4.0, 5.0));
}

#[test]
fn from_arrays_with_no_vertices_produces_empty_aabb() {
    let mesh = Mesh::from_arrays(Vec::new(), Vec::new());
    assert!(mesh.aabb.is_empty());
}

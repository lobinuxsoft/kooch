use glam::Vec3;

use crate::BlockMesh;

fn unit_cube() -> BlockMesh {
    BlockMesh::cuboid(Vec3::splat(0.5))
}

#[test]
fn to_mesh_splits_every_corner() {
    // Eight shared corners become twenty-four, one per face-corner.
    let mesh = unit_cube().to_mesh();
    assert_eq!(mesh.vertices.len(), 24);
}

#[test]
fn to_mesh_emits_twelve_triangles() {
    assert_eq!(unit_cube().to_mesh().indices.len(), 36);
}

#[test]
fn a_generated_box_shades_flat() {
    // Each face's four vertices carry one normal — sharing them would
    // average the normals and round the box's edges.
    let mesh = unit_cube().to_mesh();
    for face in mesh.vertices.chunks(4) {
        let first = face[0].normal;
        assert!(
            face.iter().all(|vertex| vertex.normal == first),
            "a face carries mixed normals: {face:?}"
        );
    }
}

#[test]
fn a_generated_box_faces_six_ways() {
    let mesh = unit_cube().to_mesh();
    let mut normals: Vec<[i32; 3]> = mesh
        .vertices
        .iter()
        .map(|vertex| vertex.normal.map(|axis| axis.round() as i32))
        .collect();
    normals.sort_unstable();
    normals.dedup();
    assert_eq!(normals.len(), 6);
}

#[test]
fn the_aabb_matches_the_extent() {
    let mesh = BlockMesh::cuboid(Vec3::new(2.0, 3.0, 4.0)).to_mesh();
    assert_eq!(mesh.aabb.min, Vec3::new(-2.0, -3.0, -4.0));
    assert_eq!(mesh.aabb.max, Vec3::new(2.0, 3.0, 4.0));
}

#[test]
fn uv_repeats_per_world_unit() {
    // A box twice as wide shows twice the texture, rather than the same
    // texels stretched across it.
    let small = BlockMesh::cuboid(Vec3::splat(0.5)).to_mesh();
    let large = BlockMesh::cuboid(Vec3::splat(1.0)).to_mesh();
    let span = |mesh: &kooch_render::mesh::Mesh| {
        let us = mesh.vertices.iter().map(|vertex| vertex.uv[0]);
        us.clone().fold(f32::MIN, f32::max) - us.fold(f32::MAX, f32::min)
    };
    assert_eq!(span(&large), span(&small) * 2.0);
}

#[test]
fn an_empty_mesh_generates_nothing() {
    let mesh = BlockMesh::default().to_mesh();
    assert!(mesh.vertices.is_empty());
    assert!(mesh.indices.is_empty());
}

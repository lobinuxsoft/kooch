use glam::Vec3;

use crate::BlockMesh;

#[test]
fn a_cuboid_collides_as_twelve_triangles() {
    let collider = BlockMesh::cuboid(Vec3::splat(0.5)).to_collider();
    assert_eq!(collider.vertices.len(), 8);
    assert_eq!(collider.indices.len(), 12);
}

#[test]
fn a_collider_keeps_corners_welded() {
    // Strictly fewer corners than the mesh drawn from the same block: a
    // seam the solver can catch on is the whole reason this is not the
    // render mesh. Counting indices would not notice — a split mesh
    // still addresses every vertex it has.
    let block = BlockMesh::cuboid(Vec3::splat(0.5));
    let collider = block.to_collider();
    assert_eq!(collider.vertices.len(), block.positions().len());
    assert!(collider.vertices.len() < block.to_mesh().vertices.len());
}

#[test]
fn a_collider_asks_for_no_hull() {
    let collider = BlockMesh::cuboid(Vec3::splat(0.5)).to_collider();
    assert!(collider.hull.points.is_empty());
    assert!(collider.parts.is_empty());
}

#[test]
fn an_empty_mesh_collides_with_nothing() {
    let collider = BlockMesh::default().to_collider();
    assert!(collider.vertices.is_empty());
    assert!(collider.indices.is_empty());
}

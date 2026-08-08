use super::*;
use crate::mesh::primitives::tests::{
    assert_outward_facing, assert_unit_normals, assert_uvs_in_unit_range,
};

#[test]
fn cylinder_fills_its_bounds_and_is_closed() {
    let mesh = cylinder(1.5, 2.0, 16);
    assert!((mesh.aabb.max.y - 2.0).abs() < 1e-4);
    assert!((mesh.aabb.min.y + 2.0).abs() < 1e-4);
    assert!((mesh.aabb.max.x - 1.5).abs() < 1e-4);
    assert_unit_normals(&mesh);
    assert_uvs_in_unit_range(&mesh);
    assert_outward_facing(&mesh);
}

/// Side normals are horizontal, cap normals vertical. If the rim
/// vertices were shared they would average into neither.
#[test]
fn cylinder_rim_normals_are_not_averaged() {
    let mesh = cylinder(1.0, 1.0, 8);
    let horizontal = mesh
        .vertices
        .iter()
        .filter(|v| v.normal[1].abs() < 1e-4)
        .count();
    let vertical = mesh
        .vertices
        .iter()
        .filter(|v| v.normal[1].abs() > 0.99)
        .count();
    assert!(horizontal > 0, "no side normals");
    assert!(vertical > 0, "no cap normals");
    assert_eq!(
        horizontal + vertical,
        mesh.vertices.len(),
        "some rim normal was averaged between the wall and a cap"
    );
}

#[test]
fn cone_has_its_apex_on_the_axis() {
    let mesh = cone(1.0, 2.0, 16);
    let apexes = mesh
        .vertices
        .iter()
        .filter(|v| (v.position[1] - 2.0).abs() < 1e-4)
        .count();
    assert_eq!(apexes, 16, "expected one apex vertex per sector");
    for v in mesh.vertices.iter().filter(|v| v.position[1] > 1.9) {
        assert!(Vec2::new(v.position[0], v.position[2]).length() < 1e-4);
    }
    assert_unit_normals(&mesh);
    assert_outward_facing(&mesh);
}

/// The side normal follows the slope. A cone normal that is merely
/// horizontal lights like a cylinder.
#[test]
fn cone_side_normals_follow_the_slope() {
    let mesh = cone(1.0, 1.0, 16);
    let sloped = mesh
        .vertices
        .iter()
        .filter(|v| v.normal[1] > 0.1 && v.normal[1] < 0.99)
        .count();
    assert!(sloped > 0, "no normal tilts with the slope");
}

#[test]
fn degenerate_dimensions_are_clamped() {
    for mesh in [cylinder(0.0, 0.0, 0), cone(0.0, 0.0, 0)] {
        assert!(mesh.index_count() > 0);
        assert_outward_facing(&mesh);
    }
}

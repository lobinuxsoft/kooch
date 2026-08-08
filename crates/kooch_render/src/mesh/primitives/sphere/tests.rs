use super::*;
use crate::mesh::primitives::tests::{
    assert_outward_facing, assert_unit_normals, assert_uvs_in_unit_range,
};

#[test]
fn sphere_points_all_sit_on_the_radius() {
    let mesh = sphere(2.0, 8, 12);
    for v in &mesh.vertices {
        let r = Vec3::from_array(v.position).length();
        assert!((r - 2.0).abs() < 1e-4, "vertex off the sphere: r = {r}");
    }
    assert_unit_normals(&mesh);
    assert_uvs_in_unit_range(&mesh);
    assert_outward_facing(&mesh);
}

/// On a sphere the normal *is* the direction from the centre, so this
/// catches an inverted or facet-averaged normal in one assertion.
#[test]
fn sphere_normals_point_straight_out() {
    let mesh = sphere(1.0, 8, 12);
    for v in &mesh.vertices {
        let p = Vec3::from_array(v.position);
        let n = Vec3::from_array(v.normal);
        if p.length() > 1e-3 {
            assert!(
                n.dot(p.normalize()) > 0.99,
                "normal does not agree with position: {n:?} vs {p:?}"
            );
        }
    }
}

#[test]
fn capsule_total_height_is_body_plus_both_caps() {
    let mesh = capsule(0.5, 1.0, 8, 12);
    let height = mesh.aabb.max.y - mesh.aabb.min.y;
    assert!(
        (height - 3.0).abs() < 1e-3,
        "expected 2*(1.0 + 0.5) = 3.0, got {height}"
    );
    let width = mesh.aabb.max.x - mesh.aabb.min.x;
    assert!(
        (width - 1.0).abs() < 1e-3,
        "expected diameter 1.0, got {width}"
    );
    assert_unit_normals(&mesh);
    assert_outward_facing(&mesh);
}

/// A zero-height capsule is a sphere, not a degenerate shape — the
/// Inspector passes through it while the user types.
#[test]
fn capsule_with_no_body_is_a_sphere() {
    let mesh = capsule(1.0, 0.0, 8, 12);
    let height = mesh.aabb.max.y - mesh.aabb.min.y;
    assert!((height - 2.0).abs() < 1e-3, "got {height}");
    for v in &mesh.vertices {
        assert!(Vec3::from_array(v.position).length() < 1.001);
    }
}

#[test]
fn degenerate_segment_counts_are_clamped() {
    let mesh = sphere(1.0, 0, 0);
    assert!(
        mesh.index_count() > 0,
        "a sphere with no segments has no surface"
    );
    assert_outward_facing(&mesh);
}

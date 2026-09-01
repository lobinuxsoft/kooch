use super::*;

fn tetrahedron() -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let vertices = vec![
        Vec3::ZERO,
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
    ];
    let indices = vec![[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]];
    (vertices, indices)
}

/// Every variant has to reach a rapier collider, or the Inspector offers
/// a shape that quietly does nothing.
#[test]
fn every_analytic_shape_builds() {
    let shapes = [
        CollisionShape::Sphere { radius: 1.0 },
        CollisionShape::Cuboid {
            half_extents: Vec3::ONE,
        },
        CollisionShape::Capsule {
            radius: 1.0,
            half_height: 1.0,
        },
        CollisionShape::Cylinder {
            radius: 1.0,
            half_height: 1.0,
        },
        CollisionShape::RoundCylinder {
            radius: 1.0,
            half_height: 1.0,
            border_radius: 0.1,
        },
        CollisionShape::Cone {
            radius: 1.0,
            half_height: 1.0,
        },
        CollisionShape::HalfSpace { normal: Vec3::Y },
        CollisionShape::Segment {
            a: Vec3::ZERO,
            b: Vec3::Y,
        },
        CollisionShape::Triangle {
            a: Vec3::ZERO,
            b: Vec3::X,
            c: Vec3::Y,
        },
    ];
    for shape in shapes {
        assert!(shape_builder(&shape).is_ok(), "{} refused", shape.name());
    }
}

#[test]
fn every_mesh_shape_builds() {
    let (vertices, indices) = tetrahedron();
    let shapes = [
        CollisionShape::ConvexHull {
            points: vertices.clone(),
        },
        CollisionShape::ConvexDecomposition {
            vertices: vertices.clone(),
            indices: indices.clone(),
        },
        CollisionShape::TriMesh {
            vertices: vertices.clone(),
            indices: indices.clone(),
        },
        CollisionShape::Polyline {
            vertices: vertices.clone(),
        },
        CollisionShape::VoxelizedMesh {
            vertices,
            indices,
            size: 0.25,
            solid: true,
        },
        CollisionShape::Voxels {
            size: Vec3::splat(1.0),
            cells: vec![glam::IVec3::ZERO],
        },
        CollisionShape::Heightfield {
            heights: vec![0.0; 9],
            rows: 3,
            cols: 3,
            scale: Vec3::ONE,
        },
    ];
    for shape in shapes {
        assert!(shape_builder(&shape).is_ok(), "{} refused", shape.name());
    }
}

/// A degenerate point set has to be reported rather than silently
/// producing a collider with no volume.
#[test]
fn a_flat_hull_is_refused() {
    let collinear = vec![Vec3::ZERO, Vec3::X, Vec3::X * 2.0];
    assert_eq!(
        shape_builder(&CollisionShape::ConvexHull { points: collinear }).err(),
        Some(ShapeError::DegenerateHull)
    );
}

#[test]
fn an_empty_mesh_is_refused() {
    assert_eq!(
        shape_builder(&CollisionShape::TriMesh {
            vertices: Vec::new(),
            indices: Vec::new(),
        })
        .err(),
        Some(ShapeError::NoGeometry)
    );
}

/// `Array2::new` asserts, and an assert inside the solver is a panic with
/// no author-facing cause.
#[test]
fn a_ragged_grid_is_refused() {
    assert_eq!(
        shape_builder(&CollisionShape::Heightfield {
            heights: vec![0.0; 5],
            rows: 3,
            cols: 3,
            scale: Vec3::ONE,
        })
        .err(),
        Some(ShapeError::RaggedHeightfield)
    );
}

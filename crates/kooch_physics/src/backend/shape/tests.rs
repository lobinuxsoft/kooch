use super::*;

#[test]
fn a_box_scales_per_axis() {
    let scaled = CollisionShape::Cuboid {
        half_extents: Vec3::ONE,
    }
    .scaled(Vec3::new(2.0, 3.0, 4.0));
    assert_eq!(
        scaled,
        CollisionShape::Cuboid {
            half_extents: Vec3::new(2.0, 3.0, 4.0)
        }
    );
}

/// A capsule scaled on Y gets taller, not fatter — the convention every
/// engine uses, and the one the collider gizmo mirrors.
#[test]
fn a_capsule_takes_height_from_y() {
    let scaled = CollisionShape::Capsule {
        radius: 1.0,
        half_height: 1.0,
    }
    .scaled(Vec3::new(1.0, 4.0, 1.0));
    assert_eq!(
        scaled,
        CollisionShape::Capsule {
            radius: 1.0,
            half_height: 4.0
        }
    );
}

/// The ground must not tilt under a non-uniformly scaled entity: a normal
/// transforms by the inverse, not like a point.
#[test]
fn a_plane_keeps_its_facing() {
    let CollisionShape::HalfSpace { normal } =
        CollisionShape::HalfSpace { normal: Vec3::Y }.scaled(Vec3::new(10.0, 1.0, 10.0))
    else {
        panic!("expected a half-space");
    };
    assert!(normal.abs_diff_eq(Vec3::Y, 1e-6));
}

#[test]
fn a_hull_scales_every_point() {
    let scaled = CollisionShape::ConvexHull {
        part: ConvexPart {
            points: vec![Vec3::X, Vec3::Y],
            faces: vec![[0, 1, 0]],
        },
    }
    .scaled(Vec3::splat(2.0));
    assert_eq!(
        scaled,
        CollisionShape::ConvexHull {
            part: ConvexPart {
                points: vec![Vec3::X * 2.0, Vec3::Y * 2.0],
                // The topology survives: a positive diagonal scale maps a
                // convex hull to a convex hull with the same faces, so a
                // claim that was true stays true.
                faces: vec![[0, 1, 0]],
            },
        }
    );
}

/// A dimension mid-edit passes through zero, and a zero-sized shape makes
/// the solver produce NaNs that outlive the typo.
#[test]
fn a_zero_dimension_is_clamped() {
    let scaled = CollisionShape::Sphere { radius: 1.0 }.scaled(Vec3::ZERO);
    assert_eq!(scaled, CollisionShape::Sphere { radius: MIN_EXTENT });
}

#[test]
fn points_land_in_their_cells() {
    let shape = CollisionShape::voxels_from_points(
        Vec3::splat(1.0),
        &[
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(0.9, 0.1, 0.2),
            Vec3::new(2.5, 0.0, 0.0),
        ],
    );
    let CollisionShape::Voxels { cells, .. } = shape else {
        panic!("expected voxels");
    };
    assert_eq!(
        cells,
        vec![IVec3::ZERO, IVec3::new(2, 0, 0)],
        "same cell, said once"
    );
}

/// A dynamic body on a shape with no volume gets no inertia tensor, which
/// is what the mass path keys on.
#[test]
fn hollow_shapes_report_hollow() {
    assert!(CollisionShape::Sphere { radius: 1.0 }.is_solid());
    assert!(!CollisionShape::HalfSpace { normal: Vec3::Y }.is_solid());
    assert!(
        !CollisionShape::TriMesh {
            vertices: Vec::new(),
            indices: Vec::new()
        }
        .is_solid()
    );
}

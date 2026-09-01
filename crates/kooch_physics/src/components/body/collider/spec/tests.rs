use super::*;

use crate::components::Collider;

fn mesh() -> ColliderMesh {
    ColliderMesh {
        vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z],
        indices: vec![[0, 1, 2], [0, 2, 3]],
    }
}

fn cached(guid: Guid) -> ColliderMeshCache {
    let mut cache = ColliderMeshCache::new();
    cache.insert(guid, mesh());
    cache
}

fn shaped(shape: u32, mesh: Option<Guid>) -> ShapeSpec {
    Collider {
        shape,
        mesh,
        ..Default::default()
    }
    .shape_spec(None)
}

#[test]
fn the_analytic_shapes_need_no_mesh() {
    assert!(matches!(
        shaped(SHAPE_CONE, None).resolve(None),
        Some(CollisionShape::Cone { .. })
    ));
    assert!(matches!(
        shaped(SHAPE_ROUND_CYLINDER, None).resolve(None),
        Some(CollisionShape::RoundCylinder { .. })
    ));
}

/// Substituting a unit sphere for a level's collision would be a floor
/// nobody authored, in a place nobody looks.
#[test]
fn a_missing_mesh_resolves_to_nothing() {
    let spec = shaped(SHAPE_TRIMESH, Some(Guid::new_v4()));
    assert_eq!(spec.resolve(None), None);
    assert!(spec.awaits_mesh(None));
}

#[test]
fn a_cached_mesh_becomes_geometry() {
    let guid = Guid::new_v4();
    let cache = cached(guid);
    let collider = Collider {
        shape: SHAPE_TRIMESH,
        mesh: Some(guid),
        ..Default::default()
    };
    let spec = collider.shape_spec(Some(&cache));
    assert!(!spec.awaits_mesh(Some(&cache)));
    assert_eq!(
        spec.resolve(Some(&cache)),
        Some(CollisionShape::TriMesh {
            vertices: mesh().vertices,
            indices: mesh().indices,
        })
    );
}

/// The epoch is the whole reason a body authored before its mesh loaded
/// ever gets rebuilt: nothing else about the spec changes.
#[test]
fn the_epoch_reaches_the_spec() {
    let guid = Guid::new_v4();
    let collider = Collider {
        shape: SHAPE_CONVEX_HULL,
        mesh: Some(guid),
        ..Default::default()
    };
    let before = collider.shape_spec(None);
    let after = collider.shape_spec(Some(&cached(guid)));
    assert_ne!(before, after, "the arrival has to retire the old body");
}

/// A point cloud has no triangles. A hull is happy with that; a
/// decomposition and a trimesh are not, and must say so rather than hand
/// the solver an empty index buffer.
#[test]
fn topology_free_meshes_only_feed_a_hull() {
    let guid = Guid::new_v4();
    let mut cache = ColliderMeshCache::new();
    cache.insert(
        guid,
        ColliderMesh {
            vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z],
            indices: Vec::new(),
        },
    );
    assert!(
        shaped(SHAPE_CONVEX_HULL, Some(guid))
            .resolve(Some(&cache))
            .is_some()
    );
    assert!(
        shaped(SHAPE_TRIMESH, Some(guid))
            .resolve(Some(&cache))
            .is_none()
    );
    assert!(
        shaped(SHAPE_CONVEX_DECOMPOSITION, Some(guid))
            .resolve(Some(&cache))
            .is_none()
    );
}

/// A scene authored in a newer editor loads and collides with something,
/// rather than dropping its colliders on the floor.
#[test]
fn an_unknown_shape_falls_back() {
    assert!(matches!(
        shaped(9999, None).resolve(None),
        Some(CollisionShape::Sphere { .. })
    ));
}

/// Any entity — these tests are about the shape, not about who owns it.
fn any_entity() -> kooch_ecs::Entity {
    kooch_ecs::Entity::new(0, 0)
}
use super::*;

use crate::components::Collider;

fn mesh() -> ColliderMesh {
    ColliderMesh {
        vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z],
        indices: vec![[0, 1, 2], [0, 2, 3]],
        ..Default::default()
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
    .shape_spec(any_entity(), None)
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
    let spec = collider.shape_spec(any_entity(), Some(&cache));
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
    let before = collider.shape_spec(any_entity(), None);
    let after = collider.shape_spec(any_entity(), Some(&cached(guid)));
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
            ..Default::default()
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

/// 🔴 Every shape that ASKS for a mesh has to be able to build one.
///
/// `SHAPE_OWN_MESH` was added to `MESH_DERIVED` — so the walk fetched
/// its geometry — and never added to `from_mesh`, which falls through
/// to `_ => None`. The mesh arrived and was dropped on the last line,
/// and the block drew without colliding.
#[test]
fn every_mesh_derived_shape_builds_something() {
    use crate::backend::{ColliderMesh, ColliderMeshCache};
    use crate::components::MESH_DERIVED;

    let entity = any_entity();
    let guid = kooch_core::Guid::new_v4();
    let tetrahedron = ColliderMesh {
        vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z],
        indices: vec![[0, 1, 2], [0, 2, 3], [0, 3, 1], [1, 3, 2]],
        ..Default::default()
    };

    // Under both keys: an own-mesh shape reads the entity's, the rest
    // read the asset's, and this test is about what each BUILDS.
    let mut cache = ColliderMeshCache::new();
    cache.insert(entity, tetrahedron.clone());
    cache.insert(guid, tetrahedron);

    let unbuilt: Vec<u32> = MESH_DERIVED
        .iter()
        .copied()
        .filter(|shape| {
            let collider = Collider {
                shape: *shape,
                mesh: Some(guid),
                ..Default::default()
            };
            collider
                .shape_spec(entity, Some(&cache))
                .resolve(Some(&cache))
                .is_none()
        })
        .collect();

    assert!(
        unbuilt.is_empty(),
        "these shapes fetch a mesh and then build nothing from it: {unbuilt:?}",
    );
}

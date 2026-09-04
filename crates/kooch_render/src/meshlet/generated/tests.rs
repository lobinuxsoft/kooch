use kooch_core::Guid;

use super::GeneratedMeshes;
use crate::mesh::Aabb;
use crate::meshlet::asset::MeshletMesh;

/// A mesh with nothing in it — this store never looks inside.
fn nothing() -> MeshletMesh {
    MeshletMesh {
        vertices: Vec::new(),
        meshlet_vertices: Vec::new(),
        meshlet_triangles: Vec::new(),
        meshlets: Vec::new(),
        aabb: Aabb::empty(),
    }
}

#[test]
fn a_new_store_is_empty() {
    assert!(GeneratedMeshes::new().is_empty());
}

#[test]
fn draining_empties_the_store() {
    let mut store = GeneratedMeshes::new();
    store.insert(Guid::new_v4(), nothing());
    assert_eq!(store.drain().len(), 1);
    assert!(store.is_empty());
}

#[test]
fn a_second_edit_replaces_the_first() {
    // A drag publishes every frame; only the last one is worth uploading.
    let mut store = GeneratedMeshes::new();
    let guid = Guid::new_v4();
    store.insert(guid, nothing());
    store.insert(guid, nothing());
    assert_eq!(store.len(), 1);
}

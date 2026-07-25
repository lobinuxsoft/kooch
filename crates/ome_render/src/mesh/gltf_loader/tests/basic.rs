//! Basic loader tests: extension list, error mapping, GLB round-trip,
//! scene walk + transform bake, import-scale, and empty-document
//! rejection. URI-aware buffer resolution lives in `uri.rs`.

use super::super::{GltfMeshError, GltfMeshLoader, parse_mesh_bytes, parse_mesh_bytes_with_scale};
use super::helpers::{build_minimal_triangle_glb, build_two_translated_triangles_glb, pad_to_4};
use glam::Vec3;
use ome_core::asset_loader::{AssetError, AssetLoader, LoadContext};
use std::path::Path;

#[test]
fn extensions_includes_glb_and_gltf() {
    let loader = GltfMeshLoader;
    assert_eq!(loader.extensions(), &["glb", "gltf"]);
}

#[test]
fn invalid_bytes_return_loader_error() {
    let loader = GltfMeshLoader;
    let mut ctx = LoadContext {
        path: Path::new("bogus.glb"),
    };
    let err = loader.load(b"not a real glb", &mut ctx).unwrap_err();
    match err {
        AssetError::Loader(_) => {}
        other => panic!("expected Loader error, got {other:?}"),
    }
}

#[test]
fn minimal_glb_round_trip() {
    let glb = build_minimal_triangle_glb();
    let loader = GltfMeshLoader;
    let mut ctx = LoadContext {
        path: Path::new("triangle.glb"),
    };
    let mesh = loader
        .load(&glb, &mut ctx)
        .expect("loader should accept minimal glb");

    assert_eq!(mesh.vertex_count(), 3);
    assert_eq!(mesh.index_count(), 3);
    assert_eq!(mesh.indices, vec![0, 1, 2]);

    // Vertex positions match what we encoded.
    let positions: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.position).collect();
    assert_eq!(
        positions,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );

    // AABB envelops the triangle.
    assert_eq!(mesh.aabb.min, Vec3::new(0.0, 0.0, 0.0));
    assert_eq!(mesh.aabb.max, Vec3::new(1.0, 1.0, 0.0));
}

#[test]
fn scene_walk_concatenates_two_translated_triangles() {
    // Two-node scene: each node holds a single-triangle mesh, the
    // first translated +X by 10, the second translated -X by 10.
    // Validates: scene walk picks both, transforms are applied to
    // vertex positions, indices are rebased into the concatenated
    // pool.
    let glb = build_two_translated_triangles_glb();
    let mesh = parse_mesh_bytes(&glb).expect("multi-node load");

    assert_eq!(mesh.vertex_count(), 6, "two triangles → six vertices");
    assert_eq!(mesh.index_count(), 6);
    // Indices of the second triangle must reference vertices 3..6
    // (rebased), not 0..3 (the per-primitive local indices).
    assert_eq!(mesh.indices, vec![0, 1, 2, 3, 4, 5]);

    // First triangle's first vertex sits at local origin (0,0,0)
    // → world (+10, 0, 0). Second triangle's first vertex at world
    // (-10, 0, 0). AABB envelopes both.
    assert!(
        (mesh.vertices[0].position[0] - 10.0).abs() < 1e-4,
        "first node translated +X by 10",
    );
    assert!(
        (mesh.vertices[3].position[0] + 10.0).abs() < 1e-4,
        "second node translated -X by 10",
    );
    assert!(
        mesh.aabb.min.x < -9.0 && mesh.aabb.max.x > 9.0,
        "AABB must span both translated triangles",
    );
}

#[test]
fn import_scale_multiplies_positions() {
    // The minimal triangle spans x in [0, 1]. import_scale = 100
    // makes it span [0, 100]. Validates the scale knob applies
    // before any scene transform composes.
    let glb = build_minimal_triangle_glb();
    let mesh = parse_mesh_bytes_with_scale(&glb, 100.0).expect("load with scale");
    let xs: Vec<f32> = mesh.vertices.iter().map(|v| v.position[0]).collect();
    let max_x = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!(
        (max_x - 100.0).abs() < 1e-3,
        "scaled max X should be 100, got {max_x}",
    );
}

#[test]
fn parse_mesh_bytes_rejects_empty_document() {
    // A minimal valid glTF JSON with NO meshes.
    let json = r#"{"asset":{"version":"2.0"}}"#;
    let mut bin = Vec::new();
    bin.extend_from_slice(b"glTF"); // magic
    bin.extend_from_slice(&2u32.to_le_bytes()); // version
    let json_padded = pad_to_4(json.as_bytes());
    let total_len = 12 + 8 + json_padded.len() as u32;
    bin.extend_from_slice(&total_len.to_le_bytes());
    bin.extend_from_slice(&(json_padded.len() as u32).to_le_bytes());
    bin.extend_from_slice(b"JSON");
    bin.extend_from_slice(&json_padded);

    let err = parse_mesh_bytes(&bin).unwrap_err();
    match err {
        GltfMeshError::EmptyDocument => {}
        other => panic!("expected EmptyDocument, got {other:?}"),
    }
}

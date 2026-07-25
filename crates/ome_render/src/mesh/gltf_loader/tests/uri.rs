//! External-URI buffer tests added in #491: separate `.gltf` + sidecar
//! `.bin` resolution, embedded `data:` URI decoding, and the hygiene
//! gates that reject traversal / absolute-path / malformed payloads.

use super::helpers::{
    build_data_uri_gltf, cleanup_tmpdir, make_tmpdir, write_separate_gltf_pair,
};
use super::super::{parse_mesh_bytes_full, GltfMeshError};

#[test]
fn separate_gltf_loads_with_sidecar_buffer() {
    // Mirrors Blender's *glTF Separate* export: a `.gltf` JSON document
    // alongside a `.bin` sidecar. parse_mesh_bytes_full resolves the
    // URI relative to the document's directory.
    let dir = make_tmpdir("separate_load");
    let (gltf_path, _bin_path) = write_separate_gltf_pair(&dir, "scene", "scene.bin");

    let bytes = std::fs::read(&gltf_path).expect("read gltf");
    let mesh = parse_mesh_bytes_full(&bytes, 1.0, Some(&dir))
        .expect("sidecar buffer must resolve");

    assert_eq!(mesh.vertex_count(), 3);
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    cleanup_tmpdir(&dir);
}

#[test]
fn separate_gltf_without_base_dir_reports_unresolvable() {
    // No directory anchor → the sidecar URI cannot be located. The
    // load fails loudly so callers know to plumb the path through.
    let dir = make_tmpdir("unresolvable");
    let (gltf_path, _) = write_separate_gltf_pair(&dir, "scene", "scene.bin");
    let bytes = std::fs::read(&gltf_path).expect("read gltf");

    let err = parse_mesh_bytes_full(&bytes, 1.0, None).unwrap_err();
    assert!(
        matches!(err, GltfMeshError::BufferUriUnresolvable),
        "expected BufferUriUnresolvable, got {err:?}",
    );
    cleanup_tmpdir(&dir);
}

#[test]
fn separate_gltf_with_traversal_uri_is_rejected() {
    // A doctored .gltf claiming `"uri": "../../../etc/passwd"` must be
    // rejected before any filesystem read happens. Validates the URI
    // hygiene gate end-to-end through parse_mesh_bytes_full.
    let dir = make_tmpdir("traversal");
    let (gltf_path, _) = write_separate_gltf_pair(&dir, "scene", "../../../etc/passwd");
    let bytes = std::fs::read(&gltf_path).expect("read gltf");

    let err = parse_mesh_bytes_full(&bytes, 1.0, Some(&dir)).unwrap_err();
    match err {
        GltfMeshError::BufferUriRejected { ref uri, reason } => {
            assert_eq!(uri, "../../../etc/passwd");
            assert!(
                reason.contains(".."),
                "reason should mention `..` traversal, got `{reason}`",
            );
        }
        other => panic!("expected BufferUriRejected, got {other:?}"),
    }
    cleanup_tmpdir(&dir);
}

#[test]
fn separate_gltf_with_absolute_uri_is_rejected() {
    // POSIX absolute paths in `uri` are rejected even when `base_dir`
    // would technically allow Path::join to swallow them. Defends
    // against malicious `.gltf` files that try to read host files.
    let dir = make_tmpdir("absolute");
    let (gltf_path, _) = write_separate_gltf_pair(&dir, "scene", "/etc/passwd");
    let bytes = std::fs::read(&gltf_path).expect("read gltf");

    let err = parse_mesh_bytes_full(&bytes, 1.0, Some(&dir)).unwrap_err();
    match err {
        GltfMeshError::BufferUriRejected { ref uri, reason } => {
            assert_eq!(uri, "/etc/passwd");
            assert_eq!(reason, "absolute path");
        }
        other => panic!("expected BufferUriRejected, got {other:?}"),
    }
    cleanup_tmpdir(&dir);
}

#[test]
fn embedded_gltf_loads_with_data_uri_buffer() {
    // Mirrors Blender's *glTF Embedded* export: the binary payload
    // is base64-encoded inline in the JSON via a `data:` URI. No
    // sidecar, no filesystem touch — base_dir can be `None`.
    let json = build_data_uri_gltf();
    let mesh = parse_mesh_bytes_full(json.as_bytes(), 1.0, None)
        .expect("embedded data URI buffer must decode");

    assert_eq!(mesh.vertex_count(), 3);
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    let positions: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.position).collect();
    assert_eq!(
        positions,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
}

#[test]
fn malformed_data_uri_surfaces_specific_error() {
    // Truncated base64 payload — the decoder reports the failure
    // through `MalformedDataUri`, not a generic missing-attribute.
    let json = r#"{
  "asset": { "version": "2.0" },
  "buffers": [{ "uri": "data:application/octet-stream;base64,!!!", "byteLength": 4 }],
  "bufferViews": [],
  "accessors": [],
  "meshes": []
}"#;
    let err = parse_mesh_bytes_full(json.as_bytes(), 1.0, None).unwrap_err();
    assert!(
        matches!(err, GltfMeshError::MalformedDataUri(_)),
        "expected MalformedDataUri, got {err:?}",
    );
}

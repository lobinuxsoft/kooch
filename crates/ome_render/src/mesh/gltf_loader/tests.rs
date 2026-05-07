use super::*;
use glam::Vec3;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Builds a hand-crafted GLB containing exactly one triangle. Used by
/// the round-trip test above to exercise the full parser without
/// pulling in test-asset binaries.
pub(super) fn build_minimal_triangle_glb() -> Vec<u8> {
    // Binary chunk: indices (u32×3) | positions (vec3×3) | normals (vec3×3) | uvs (vec2×3)
    let indices: [u32; 3] = [0, 1, 2];
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let normals: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
    let uvs: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];

    let mut bin = Vec::new();
    bin.extend_from_slice(bytemuck::cast_slice(&indices));
    let positions_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&positions));
    let normals_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&normals));
    let uvs_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&uvs));
    let bin_len_unpadded = bin.len();
    // Pad bin chunk to 4 bytes (GLB spec).
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let bin_padded_len = bin.len();

    let json = format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "buffers": [{{ "byteLength": {bin_len_unpadded} }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": 12, "target": 34963 }},
    {{ "buffer": 0, "byteOffset": {positions_offset}, "byteLength": 36, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {normals_offset}, "byteLength": 36, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {uvs_offset}, "byteLength": 24, "target": 34962 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5125, "count": 3, "type": "SCALAR" }},
    {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0] }},
    {{ "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC2" }}
  ],
  "meshes": [
    {{ "primitives": [
      {{ "attributes": {{ "POSITION": 1, "NORMAL": 2, "TEXCOORD_0": 3 }}, "indices": 0 }}
    ] }}
  ]
}}"#
    );
    let json_padded = pad_to_4(json.as_bytes());

    let json_chunk_len = json_padded.len() as u32;
    let bin_chunk_len = bin_padded_len as u32;
    let total = 12 + 8 + json_chunk_len + 8 + bin_chunk_len;

    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());

    out.extend_from_slice(&json_chunk_len.to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_padded);

    out.extend_from_slice(&bin_chunk_len.to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);

    out
}

/// Two-node scene. Each node owns a single-triangle mesh; the first
/// is translated +10 along X, the second -10. Shares one vertex
/// buffer (every triangle reuses the same accessors at offset 0).
/// Used to lock the scene-walk + transform-bake + index-rebase
/// behaviour without dragging in real-asset binaries.
pub(super) fn build_two_translated_triangles_glb() -> Vec<u8> {
    let indices: [u32; 3] = [0, 1, 2];
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let normals: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
    let uvs: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];

    let mut bin = Vec::new();
    bin.extend_from_slice(bytemuck::cast_slice(&indices));
    let positions_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&positions));
    let normals_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&normals));
    let uvs_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&uvs));
    let bin_len_unpadded = bin.len();
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let bin_padded_len = bin.len();

    let json = format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "buffers": [{{ "byteLength": {bin_len_unpadded} }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": 12, "target": 34963 }},
    {{ "buffer": 0, "byteOffset": {positions_offset}, "byteLength": 36, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {normals_offset}, "byteLength": 36, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {uvs_offset}, "byteLength": 24, "target": 34962 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5125, "count": 3, "type": "SCALAR" }},
    {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0] }},
    {{ "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC2" }}
  ],
  "meshes": [
    {{ "primitives": [
      {{ "attributes": {{ "POSITION": 1, "NORMAL": 2, "TEXCOORD_0": 3 }}, "indices": 0 }}
    ] }}
  ],
  "nodes": [
    {{ "mesh": 0, "translation": [10.0, 0.0, 0.0] }},
    {{ "mesh": 0, "translation": [-10.0, 0.0, 0.0] }}
  ],
  "scenes": [
    {{ "nodes": [0, 1] }}
  ],
  "scene": 0
}}"#
    );
    let json_padded = pad_to_4(json.as_bytes());

    let json_chunk_len = json_padded.len() as u32;
    let bin_chunk_len = bin_padded_len as u32;
    let total = 12 + 8 + json_chunk_len + 8 + bin_chunk_len;

    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&json_chunk_len.to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_padded);
    out.extend_from_slice(&bin_chunk_len.to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

// ---------------------------------------------------------------------------
// External-URI buffer tests (#490)
// ---------------------------------------------------------------------------

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

/// Builds a `.gltf` JSON whose single buffer is a `data:` URI with
/// base64-encoded triangle bytes. Equivalent payload to
/// [`build_minimal_triangle_glb`] — used to exercise the embedded path
/// without touching the filesystem.
fn build_data_uri_gltf() -> String {
    use base64::Engine;

    let indices: [u32; 3] = [0, 1, 2];
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let normals: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
    let uvs: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];

    let mut bin = Vec::new();
    bin.extend_from_slice(bytemuck::cast_slice(&indices));
    let positions_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&positions));
    let normals_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&normals));
    let uvs_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&uvs));
    let bin_len = bin.len();

    let encoded = base64::engine::general_purpose::STANDARD.encode(&bin);

    format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "buffers": [{{ "uri": "data:application/octet-stream;base64,{encoded}", "byteLength": {bin_len} }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": 12, "target": 34963 }},
    {{ "buffer": 0, "byteOffset": {positions_offset}, "byteLength": 36, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {normals_offset}, "byteLength": 36, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {uvs_offset}, "byteLength": 24, "target": 34962 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5125, "count": 3, "type": "SCALAR" }},
    {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0] }},
    {{ "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC2" }}
  ],
  "meshes": [
    {{ "primitives": [
      {{ "attributes": {{ "POSITION": 1, "NORMAL": 2, "TEXCOORD_0": 3 }}, "indices": 0 }}
    ] }}
  ]
}}"#,
    )
}

// Per-test tmpdir naming: process pid + atomic counter keeps the
// names distinct even across `--test-threads=N` runs.
fn make_tmpdir(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ome_gltf_test_{}_{}_{}",
        std::process::id(),
        label,
        n,
    ));
    std::fs::create_dir_all(&dir).expect("tmpdir create");
    dir
}

fn cleanup_tmpdir(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

/// Writes a glTF *Separate* pair into `dir`: `<name>.gltf` referencing
/// the supplied URI for buffer 0, plus `<dir>/scene.bin` containing the
/// triangle's binary payload. The URI is the verbatim string the
/// document uses — useful for traversal / absolute-path hostile cases.
fn write_separate_gltf_pair(dir: &Path, name: &str, uri: &str) -> (PathBuf, PathBuf) {
    let indices: [u32; 3] = [0, 1, 2];
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let normals: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
    let uvs: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];

    let mut bin = Vec::new();
    bin.extend_from_slice(bytemuck::cast_slice(&indices));
    let positions_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&positions));
    let normals_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&normals));
    let uvs_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&uvs));
    let bin_len = bin.len();

    let json = format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "buffers": [{{ "uri": "{uri}", "byteLength": {bin_len} }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": 12, "target": 34963 }},
    {{ "buffer": 0, "byteOffset": {positions_offset}, "byteLength": 36, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {normals_offset}, "byteLength": 36, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {uvs_offset}, "byteLength": 24, "target": 34962 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5125, "count": 3, "type": "SCALAR" }},
    {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0] }},
    {{ "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC2" }}
  ],
  "meshes": [
    {{ "primitives": [
      {{ "attributes": {{ "POSITION": 1, "NORMAL": 2, "TEXCOORD_0": 3 }}, "indices": 0 }}
    ] }}
  ]
}}"#,
    );

    let gltf_path = dir.join(format!("{name}.gltf"));
    let bin_path = dir.join("scene.bin");
    std::fs::write(&gltf_path, json).expect("write gltf");
    std::fs::write(&bin_path, &bin).expect("write bin");
    (gltf_path, bin_path)
}

pub(super) fn pad_to_4(input: &[u8]) -> Vec<u8> {
    let mut padded = input.to_vec();
    // JSON chunk pads with 0x20 (space). BIN chunk pads with 0x00. We use
    // 0x20 here because this helper is shared with the JSON chunk. Bin
    // chunk pads inline above.
    while padded.len() % 4 != 0 {
        padded.push(b' ');
    }
    padded
}

//! Shared fixtures for the gltf_loader test sub-modules.
//!
//! Builders fabricate hand-crafted GLB / glTF documents so the tests
//! never depend on real-asset binaries. Filesystem helpers manage
//! per-test tmpdirs that survive `--test-threads=N` runs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Builds a `.gltf` JSON whose single buffer is a `data:` URI with
/// base64-encoded triangle bytes. Equivalent payload to
/// [`build_minimal_triangle_glb`] — used to exercise the embedded path
/// without touching the filesystem.
pub(super) fn build_data_uri_gltf() -> String {
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
pub(super) fn make_tmpdir(label: &str) -> PathBuf {
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

pub(super) fn cleanup_tmpdir(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

/// Writes a glTF *Separate* pair into `dir`: `<name>.gltf` referencing
/// the supplied URI for buffer 0, plus `<dir>/scene.bin` containing the
/// triangle's binary payload. The URI is the verbatim string the
/// document uses — useful for traversal / absolute-path hostile cases.
pub(super) fn write_separate_gltf_pair(
    dir: &Path,
    name: &str,
    uri: &str,
) -> (PathBuf, PathBuf) {
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

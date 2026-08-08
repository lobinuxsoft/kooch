use super::*;
use std::path::Path;

#[test]
fn extensions_includes_glb_and_gltf() {
    assert_eq!(MeshletMeshLoader.extensions(), &["glb", "gltf"]);
}

#[test]
fn invalid_bytes_return_loader_error() {
    let mut ctx = LoadContext {
        path: Path::new("bogus.glb"),
    };
    let err = MeshletMeshLoader
        .load(b"not a real glb", &mut ctx)
        .expect_err("garbage must fail");
    assert!(matches!(err, AssetError::Loader(_)));
}

/// End-to-end: hand-crafted minimal triangle GLB → MeshletMesh
/// with at least one meshlet covering the single triangle.
#[test]
fn minimal_glb_round_trip_to_meshlet() {
    let glb = build_minimal_triangle_glb();
    let mut ctx = LoadContext {
        path: Path::new("triangle.glb"),
    };
    let meshlet_mesh = MeshletMeshLoader
        .load(&glb, &mut ctx)
        .expect("loader should accept the minimal GLB");

    assert!(
        !meshlet_mesh.meshlets.is_empty(),
        "meshlet build must produce at least one cluster"
    );
    assert_eq!(meshlet_mesh.vertices.len(), 3);
}

/// Re-uses the same minimal GLB bytes that `mesh::gltf_loader`
/// tests build, so the two loaders share the fixture.
fn build_minimal_triangle_glb() -> Vec<u8> {
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
  ]
}}"#
    );
    let mut json_padded = json.into_bytes();
    while json_padded.len() % 4 != 0 {
        json_padded.push(b' ');
    }

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

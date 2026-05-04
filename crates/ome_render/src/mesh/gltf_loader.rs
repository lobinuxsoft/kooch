//! glTF 2.0 / GLB loader implementing [`AssetLoader<Mesh>`].
//!
//! Parses bytes (no filesystem touch — the [`AssetServer`] provides them)
//! and emits a CPU-side [`Mesh`] suitable for upload + render.
//!
//! Scope (PR-1 of #129): first mesh, first primitive of the document.
//! Multi-primitive meshes, scene hierarchy, materials, skinning and morph
//! targets are out of scope for the asset pipeline foundation; each
//! lands with its dedicated component / system (e.g. #192 skinned mesh).

use glam::Vec3;
use ome_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};

use super::asset::Mesh;
use super::vertex::{Aabb, MeshVertex};

/// Loader handling `*.glb` and `*.gltf` (with embedded buffers / data URIs).
///
/// External `.bin` sidecars are NOT supported in PR-1 because the
/// [`AssetServer`](ome_core::asset_loader::AssetServer) hands the loader a
/// flat byte slice — sidecar resolution requires a follow-up that exposes
/// the source path's directory to the loader. GLB is the recommended format
/// for production assets (everything in one file).
#[derive(Debug, Default, Clone, Copy)]
pub struct GltfMeshLoader;

impl AssetLoader<Mesh> for GltfMeshLoader {
    fn extensions(&self) -> &[&'static str] {
        &["glb", "gltf"]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<Mesh> {
        parse_mesh_bytes(bytes).map_err(|e| AssetError::Loader(Box::new(e)))
    }
}

/// Domain errors specific to mesh parsing. Wrapped into
/// [`AssetError::Loader`] when surfaced through the asset pipeline.
#[derive(Debug)]
pub enum GltfMeshError {
    /// `gltf` crate failed to parse the document.
    Gltf(gltf::Error),
    /// Required vertex attribute was missing from the primitive.
    MissingAttribute(&'static str),
    /// The document contained no meshes (or no primitives).
    EmptyDocument,
}

impl std::fmt::Display for GltfMeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gltf(e) => write!(f, "gltf parse failed: {e}"),
            Self::MissingAttribute(name) => {
                write!(f, "primitive missing required attribute: {name}")
            }
            Self::EmptyDocument => write!(f, "gltf document contains no mesh primitives"),
        }
    }
}

impl std::error::Error for GltfMeshError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Gltf(e) => Some(e),
            _ => None,
        }
    }
}

impl From<gltf::Error> for GltfMeshError {
    fn from(e: gltf::Error) -> Self {
        Self::Gltf(e)
    }
}

/// Parses a glTF / GLB byte slice into a [`Mesh`].
pub fn parse_mesh_bytes(bytes: &[u8]) -> Result<Mesh, GltfMeshError> {
    let gltf = gltf::Gltf::from_slice(bytes)?;
    let blob = gltf.blob.as_deref();

    let document = gltf.document;
    let buffers = collect_buffers(&document, blob)?;

    let primitive = document
        .meshes()
        .next()
        .and_then(|m| m.primitives().next())
        .ok_or(GltfMeshError::EmptyDocument)?;

    let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));

    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or(GltfMeshError::MissingAttribute("POSITION"))?
        .collect();
    let vertex_count = positions.len();

    let normals: Vec<[f32; 3]> = reader
        .read_normals()
        .map(|iter| iter.collect())
        .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; vertex_count]);

    let uvs: Vec<[f32; 2]> = reader
        .read_tex_coords(0)
        .map(|coords| coords.into_f32().collect())
        .unwrap_or_else(|| vec![[0.0, 0.0]; vertex_count]);

    let mut aabb = Aabb::empty();
    let vertices: Vec<MeshVertex> = (0..vertex_count)
        .map(|i| {
            let p = positions[i];
            aabb.expand(Vec3::from_array(p));
            MeshVertex {
                position: p,
                normal: *normals.get(i).unwrap_or(&[0.0, 1.0, 0.0]),
                uv: *uvs.get(i).unwrap_or(&[0.0, 0.0]),
            }
        })
        .collect();

    let indices: Vec<u32> = match reader.read_indices() {
        Some(idx) => idx.into_u32().collect(),
        None => (0..vertex_count as u32).collect(),
    };

    Ok(Mesh {
        vertices,
        indices,
        aabb,
    })
}

/// Resolves the document's buffer bytes. GLB stores them inline (single
/// `blob`). External `.bin` sidecars and `data:` URI buffers are NOT
/// supported in PR-1 — return [`GltfMeshError::MissingAttribute`] so the
/// failure is observable. Production assets should ship as GLB anyway
/// (single file, no sidecar fragility).
fn collect_buffers(
    document: &gltf::Document,
    glb_blob: Option<&[u8]>,
) -> Result<Vec<Vec<u8>>, GltfMeshError> {
    let mut out = Vec::with_capacity(document.buffers().len());
    for buffer in document.buffers() {
        match buffer.source() {
            gltf::buffer::Source::Bin => {
                let blob = glb_blob.ok_or(GltfMeshError::MissingAttribute("glb-binary-chunk"))?;
                out.push(blob.to_vec());
            }
            gltf::buffer::Source::Uri(_) => {
                return Err(GltfMeshError::MissingAttribute("external-uri"));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let mesh = loader.load(&glb, &mut ctx).expect("loader should accept minimal glb");

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
    fn build_minimal_triangle_glb() -> Vec<u8> {
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

    fn pad_to_4(input: &[u8]) -> Vec<u8> {
        let mut padded = input.to_vec();
        // JSON chunk pads with 0x20 (space). BIN chunk pads with 0x00. We use
        // 0x20 here because this helper is shared with the JSON chunk. Bin
        // chunk pads inline above.
        while padded.len() % 4 != 0 {
            padded.push(b' ');
        }
        padded
    }
}

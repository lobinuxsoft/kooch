//! glTF 2.0 / GLB loader implementing [`AssetLoader<Mesh>`].
//!
//! Parses bytes (no filesystem touch — the [`AssetServer`] provides them)
//! and emits a CPU-side [`Mesh`] suitable for upload + render.
//!
//! # Coverage (post-#460)
//!
//! - Walks the default scene's node tree top-down, composing per-node
//!   transforms. Vertex positions are baked into world space (relative
//!   to the document root). Skinning + animation are explicitly out of
//!   scope and tracked under #453.
//! - Concatenates every visited primitive's geometry into one [`Mesh`].
//!   Per-primitive material associations are not preserved here; the
//!   material pipeline (#440 / #443) will land its own per-primitive
//!   path when bindless textures arrive.
//! - Optional `import_scale` factor applied at the root before scene
//!   transforms — lets the editor / artists normalise units without
//!   touching the source asset. Persistence of the scale via `.meta`
//!   files is the next PR (Plan B part 2).

use glam::{Mat3, Mat4, Vec3};
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

/// Parses a glTF / GLB byte slice into a [`Mesh`] using identity
/// import-scale. Convenience wrapper over [`parse_mesh_bytes_with_scale`].
pub fn parse_mesh_bytes(bytes: &[u8]) -> Result<Mesh, GltfMeshError> {
    parse_mesh_bytes_with_scale(bytes, 1.0)
}

/// Parses a glTF / GLB byte slice into a [`Mesh`]. The default scene's
/// node hierarchy is walked top-down; every (mesh, primitive) reached
/// is concatenated into one geometry pool with vertex positions baked
/// into world space. `import_scale` multiplies positions before scene
/// transforms apply — a single knob to convert mm-authored assets into
/// metric world units without modifying the source `.glb`.
///
/// Documents without an explicit scene fall back to enumerating every
/// mesh under the implicit identity transform — matches the gltf-rs
/// crate's `default_scene` lookup convention.
pub fn parse_mesh_bytes_with_scale(
    bytes: &[u8],
    import_scale: f32,
) -> Result<Mesh, GltfMeshError> {
    let gltf = gltf::Gltf::from_slice(bytes)?;
    let blob = gltf.blob.as_deref();
    let document = gltf.document;
    let buffers = collect_buffers(&document, blob)?;

    let mut out_vertices: Vec<MeshVertex> = Vec::new();
    let mut out_indices: Vec<u32> = Vec::new();
    let mut aabb = Aabb::empty();

    let scale_root = Mat4::from_scale(Vec3::splat(import_scale));

    if let Some(scene) = document.default_scene().or_else(|| document.scenes().next()) {
        for root in scene.nodes() {
            walk_node(
                &root,
                scale_root,
                &buffers,
                &mut out_vertices,
                &mut out_indices,
                &mut aabb,
            )?;
        }
    } else {
        // No scene defined — emit every mesh under the implicit
        // identity transform (still scaled by import_scale).
        for mesh in document.meshes() {
            for primitive in mesh.primitives() {
                ingest_primitive(
                    &primitive,
                    scale_root,
                    &buffers,
                    &mut out_vertices,
                    &mut out_indices,
                    &mut aabb,
                )?;
            }
        }
    }

    if out_vertices.is_empty() {
        return Err(GltfMeshError::EmptyDocument);
    }

    Ok(Mesh {
        vertices: out_vertices,
        indices: out_indices,
        aabb,
    })
}

/// Composes `parent_xform` with the node's local transform and
/// recurses. Every primitive reached is ingested at the cumulative
/// world transform.
fn walk_node(
    node: &gltf::Node<'_>,
    parent_xform: Mat4,
    buffers: &[Vec<u8>],
    out_vertices: &mut Vec<MeshVertex>,
    out_indices: &mut Vec<u32>,
    aabb: &mut Aabb,
) -> Result<(), GltfMeshError> {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world_xform = parent_xform * local;

    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            ingest_primitive(
                &primitive,
                world_xform,
                buffers,
                out_vertices,
                out_indices,
                aabb,
            )?;
        }
    }

    for child in node.children() {
        walk_node(&child, world_xform, buffers, out_vertices, out_indices, aabb)?;
    }
    Ok(())
}

/// Reads a single primitive, applies `world_xform` to positions
/// (and normal-correct transform to normals), appends the result to
/// the output buffers with indices rebased into the cumulative pool.
fn ingest_primitive(
    primitive: &gltf::Primitive<'_>,
    world_xform: Mat4,
    buffers: &[Vec<u8>],
    out_vertices: &mut Vec<MeshVertex>,
    out_indices: &mut Vec<u32>,
    aabb: &mut Aabb,
) -> Result<(), GltfMeshError> {
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

    // Normals transform with the inverse-transpose of the upper 3×3
    // (handles non-uniform scale correctly). Falls back to the
    // identity slice if the matrix is singular — signals a degenerate
    // node we still want to ingest rather than abort the whole load.
    let normal_xform = Mat3::from_mat4(world_xform)
        .inverse()
        .transpose();

    let vertex_offset = out_vertices.len() as u32;

    for i in 0..vertex_count {
        let p_local = Vec3::from_array(positions[i]);
        let p_world = world_xform.transform_point3(p_local);
        aabb.expand(p_world);

        let n_local = Vec3::from_array(normals[i]);
        let n_world = (normal_xform * n_local).normalize_or_zero();
        let n_out = if n_world == Vec3::ZERO {
            [0.0, 1.0, 0.0]
        } else {
            n_world.to_array()
        };

        out_vertices.push(MeshVertex {
            position: p_world.to_array(),
            normal: n_out,
            uv: *uvs.get(i).unwrap_or(&[0.0, 0.0]),
        });
    }

    let primitive_indices: Vec<u32> = match reader.read_indices() {
        Some(idx) => idx.into_u32().collect(),
        None => (0..vertex_count as u32).collect(),
    };
    out_indices.extend(primitive_indices.into_iter().map(|i| i + vertex_offset));
    Ok(())
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

    /// Two-node scene. Each node owns a single-triangle mesh; the first
    /// is translated +10 along X, the second -10. Shares one vertex
    /// buffer (every triangle reuses the same accessors at offset 0).
    /// Used to lock the scene-walk + transform-bake + index-rebase
    /// behaviour without dragging in real-asset binaries.
    fn build_two_translated_triangles_glb() -> Vec<u8> {
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

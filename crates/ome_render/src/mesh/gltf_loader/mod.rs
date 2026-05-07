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

use glam::{Mat4, Vec3};
use ome_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};

use super::asset::Mesh;
use super::vertex::{Aabb, MeshVertex};

mod buffers;
mod walk;

#[cfg(test)]
mod tests;

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
    let buffers = buffers::collect_buffers(&document, blob)?;

    let mut out_vertices: Vec<MeshVertex> = Vec::new();
    let mut out_indices: Vec<u32> = Vec::new();
    let mut aabb = Aabb::empty();

    let scale_root = Mat4::from_scale(Vec3::splat(import_scale));

    if let Some(scene) = document.default_scene().or_else(|| document.scenes().next()) {
        for root in scene.nodes() {
            walk::walk_node(
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
                walk::ingest_primitive(
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

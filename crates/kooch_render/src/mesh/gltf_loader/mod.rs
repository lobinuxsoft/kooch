//! glTF 2.0 / GLB loader implementing [`AssetLoader<Mesh>`].

use std::path::Path;

use glam::{Mat4, Vec3};
use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};

use super::asset::Mesh;
use super::vertex::{Aabb, MeshVertex};

mod buffers;
mod data_uri;
mod walk;

#[cfg(test)]
mod tests;

/// Loader handling `*.glb` and `*.gltf`. GLB packages every buffer in the same file; `.gltf`
/// documents may reference external buffers either as sidecar files (relative path) or inline
/// `data:` URIs — both resolved through [`LoadContext::path`].
#[derive(Debug, Default, Clone, Copy)]
pub struct GltfMeshLoader;

impl AssetLoader<Mesh> for GltfMeshLoader {
    fn extensions(&self) -> &[&'static str] {
        &["glb", "gltf"]
    }

    fn load(&self, bytes: &[u8], ctx: &mut LoadContext<'_>) -> AssetResult<Mesh> {
        parse_mesh_bytes_full(bytes, 1.0, ctx.path.parent())
            .map_err(|e| AssetError::Loader(Box::new(e)))
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
    /// External buffer URI requested but no base directory was
    /// supplied (bytes loaded from memory have no anchor for
    /// relative-path resolution).
    BufferUriUnresolvable,
    /// Buffer URI rejected for hygiene reasons (absolute path, `..`
    /// traversal, unsupported scheme).
    BufferUriRejected { uri: String, reason: &'static str },
    /// Filesystem read failed for a sidecar buffer.
    BufferIo { uri: String, source: std::io::Error },
    /// `data:` URI buffer was present but its payload could not be
    /// decoded (missing separator, unsupported encoding, malformed
    /// base64). The static reason describes which gate tripped.
    MalformedDataUri(&'static str),
}

impl std::fmt::Display for GltfMeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gltf(e) => write!(f, "gltf parse failed: {e}"),
            Self::MissingAttribute(name) => {
                write!(f, "primitive missing required attribute: {name}")
            }
            Self::EmptyDocument => write!(f, "gltf document contains no mesh primitives"),
            Self::BufferUriUnresolvable => write!(
                f,
                "external buffer URI cannot be resolved without a document directory",
            ),
            Self::BufferUriRejected { uri, reason } => {
                write!(f, "buffer URI rejected ({reason}): {uri}")
            }
            Self::BufferIo { uri, source } => {
                write!(f, "failed to read buffer at {uri}: {source}")
            }
            Self::MalformedDataUri(reason) => write!(f, "malformed data URI ({reason})"),
        }
    }
}

impl std::error::Error for GltfMeshError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Gltf(e) => Some(e),
            Self::BufferIo { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<gltf::Error> for GltfMeshError {
    fn from(e: gltf::Error) -> Self {
        Self::Gltf(e)
    }
}

/// Parses a glTF / GLB byte slice into a [`Mesh`] using identity import-scale and no base
/// directory.
pub fn parse_mesh_bytes(bytes: &[u8]) -> Result<Mesh, GltfMeshError> {
    parse_mesh_bytes_full(bytes, 1.0, None)
}

/// Parses a glTF / GLB byte slice into a [`Mesh`] with a custom
/// import scale. Memory-only convenience over [`parse_mesh_bytes_full`].
pub fn parse_mesh_bytes_with_scale(bytes: &[u8], import_scale: f32) -> Result<Mesh, GltfMeshError> {
    parse_mesh_bytes_full(bytes, import_scale, None)
}

/// Parses a glTF / GLB byte slice into a [`Mesh`]. The default scene's node hierarchy is walked
/// top-down; every (mesh, primitive) reached is concatenated into one geometry pool with vertex
/// positions baked into world space.
pub fn parse_mesh_parts(
    bytes: &[u8],
    base_dir: Option<&Path>,
) -> Result<Vec<(Vec<Vec3>, Vec<[u32; 3]>)>, GltfMeshError> {
    let gltf = gltf::Gltf::from_slice(bytes)?;
    let blob = gltf.blob.as_deref();
    let document = gltf.document;
    let buffers = buffers::collect_buffers(&document, blob, base_dir)?;

    let mut parts = Vec::new();
    match document
        .default_scene()
        .or_else(|| document.scenes().next())
    {
        Some(scene) => {
            for root in scene.nodes() {
                walk::walk_parts(&root, Mat4::IDENTITY, &buffers, &mut parts)?;
            }
        }
        None => {
            for mesh in document.meshes() {
                for primitive in mesh.primitives() {
                    parts.push(walk::primitive_geometry(
                        &primitive,
                        Mat4::IDENTITY,
                        &buffers,
                    )?);
                }
            }
        }
    }
    parts.retain(|(points, _)| !points.is_empty());
    Ok(parts)
}

pub fn parse_mesh_bytes_full(
    bytes: &[u8],
    import_scale: f32,
    base_dir: Option<&Path>,
) -> Result<Mesh, GltfMeshError> {
    let gltf = gltf::Gltf::from_slice(bytes)?;
    let blob = gltf.blob.as_deref();
    let document = gltf.document;
    let buffers = buffers::collect_buffers(&document, blob, base_dir)?;

    let mut out_vertices: Vec<MeshVertex> = Vec::new();
    let mut out_indices: Vec<u32> = Vec::new();
    let mut aabb = Aabb::empty();

    let scale_root = Mat4::from_scale(Vec3::splat(import_scale));

    if let Some(scene) = document
        .default_scene()
        .or_else(|| document.scenes().next())
    {
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

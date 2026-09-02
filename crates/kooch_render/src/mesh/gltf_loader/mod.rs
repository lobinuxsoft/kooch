//! glTF 2.0 / GLB loader implementing [`AssetLoader<Mesh>`].
//!
//! Parses bytes (the [`AssetServer`] reads them from disk) and emits
//! a CPU-side [`Mesh`] suitable for upload + render. The source path
//! travels in [`LoadContext::path`] so external buffer URIs can be
//! resolved relative to the document — sidecar `.bin` files for
//! glTF *Separate*, base64 payloads for *Embedded*.
//!
//! # Coverage
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
//! - External buffer URIs (sidecar / data:) resolved when the loader
//!   has a base directory; rejected with hygiene failures for absolute
//!   paths, `..` traversal, or non-`data:` schemes.

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

/// Loader handling `*.glb` and `*.gltf`. GLB packages every buffer in
/// the same file; `.gltf` documents may reference external buffers
/// either as sidecar files (relative path) or inline `data:` URIs —
/// both resolved through [`LoadContext::path`].
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

/// Parses a glTF / GLB byte slice into a [`Mesh`] using identity
/// import-scale and no base directory. GLB documents work; `.gltf`
/// documents that reference external buffers will fail because there
/// is no directory to resolve them against — load via
/// [`parse_mesh_bytes_full`] when sidecars or filesystem-relative
/// paths matter.
pub fn parse_mesh_bytes(bytes: &[u8]) -> Result<Mesh, GltfMeshError> {
    parse_mesh_bytes_full(bytes, 1.0, None)
}

/// Parses a glTF / GLB byte slice into a [`Mesh`] with a custom
/// import scale. Memory-only convenience over [`parse_mesh_bytes_full`].
pub fn parse_mesh_bytes_with_scale(bytes: &[u8], import_scale: f32) -> Result<Mesh, GltfMeshError> {
    parse_mesh_bytes_full(bytes, import_scale, None)
}

/// Parses a glTF / GLB byte slice into a [`Mesh`]. The default scene's
/// node hierarchy is walked top-down; every (mesh, primitive) reached
/// is concatenated into one geometry pool with vertex positions baked
/// into world space. `import_scale` multiplies positions before scene
/// transforms apply — a single knob to convert mm-authored assets into
/// metric world units without modifying the source `.glb`.
///
/// `base_dir` anchors external buffer URIs (sidecar `.bin` files).
/// When `Some`, sidecar URIs resolve against it; when `None`, any
/// non-GLB buffer reference fails with [`GltfMeshError::BufferUriUnresolvable`].
///
/// Documents without an explicit scene fall back to enumerating every
/// mesh under the implicit identity transform — matches the gltf-rs
/// crate's `default_scene` lookup convention.
/// Every primitive's positions, kept apart, in scene space.
///
/// What [`parse_mesh_bytes_full`] flattens, this keeps separate — the one
/// case that needs it is a baked convex decomposition, where each
/// primitive *is* one convex piece and merging them would give back the
/// concave solid the decomposition exists to avoid.
///
/// Positions only. A collider has no use for normals or UVs, and a piece
/// is consumed as a point cloud.
pub fn parse_mesh_parts(
    bytes: &[u8],
    base_dir: Option<&Path>,
) -> Result<Vec<Vec<Vec3>>, GltfMeshError> {
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
                    parts.push(walk::primitive_points(
                        &primitive,
                        Mat4::IDENTITY,
                        &buffers,
                    )?);
                }
            }
        }
    }
    parts.retain(|part| !part.is_empty());
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

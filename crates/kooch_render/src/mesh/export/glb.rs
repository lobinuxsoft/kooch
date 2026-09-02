//! [`to_glb`] — writes a [`Mesh`] out as a self-contained `.glb`.
//!
//! # Why this exists beyond baking primitives
//!
//! The engine could generate primitives straight into memory and never
//! write a file. The export path earns its keep on the *other* job:
//! turning a heavy visual mesh into a simplified collision mesh an artist
//! can open, look at, adjust, and hand back as a collider source (#137).
//! A collision mesh nobody can see is a collision mesh nobody trusts.
//!
//! # No new dependencies
//!
//! The `gltf` crate already vendors `gltf_json` as `gltf::json` (serde
//! `Serialize`), and `gltf::binary::Glb` writes the container with its
//! chunk framing and 4-byte padding. So the engine gains an exporter
//! without gaining a dependency, and it round-trips through the very
//! importer it has to stay compatible with.

use std::borrow::Cow;
use std::mem;

use gltf::json;
use json::validation::Checked::Valid;
use json::validation::USize64;

use crate::mesh::{Mesh, MeshVertex};

/// Why an export failed.
#[derive(Debug)]
pub enum ExportError {
    /// Nothing to write. An empty `.glb` is not a valid glTF asset, and
    /// silently producing one turns a generator bug into a mystery at
    /// import time.
    Empty,
    /// The index list is not a whole number of triangles.
    PartialTriangle(usize),
    /// An index addresses a vertex that does not exist.
    IndexOutOfRange { index: u32, vertices: u32 },
    /// Serialising the JSON chunk failed.
    Json(String),
    /// Writing the GLB container failed.
    Container(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Empty => write!(f, "mesh has no vertices or no indices"),
            ExportError::PartialTriangle(n) => {
                write!(f, "index count {n} is not a multiple of 3")
            }
            ExportError::IndexOutOfRange { index, vertices } => {
                write!(f, "index {index} addresses {vertices} vertices")
            }
            ExportError::Json(e) => write!(f, "failed to serialise glTF JSON: {e}"),
            ExportError::Container(e) => write!(f, "failed to write GLB container: {e}"),
        }
    }
}

impl std::error::Error for ExportError {}

/// Serialises `mesh` as a self-contained binary glTF.
///
/// One buffer, one mesh, one primitive: positions, normals, UVs and
/// indices. `name` becomes the glTF mesh and node name, which is what an
/// external viewer shows in its outliner.
///
/// The vertex stream goes out interleaved, exactly as [`MeshVertex`] is
/// laid out in memory, with three accessors reading it at different byte
/// offsets and a shared stride. That is glTF's intended layout for
/// interleaved data, and it means the bytes are copied once rather than
/// de-interleaved into three arrays.
pub fn to_glb(mesh: &Mesh, name: &str) -> Result<Vec<u8>, ExportError> {
    to_glb_parts(&[(mesh, name)])
}

/// Serialises several meshes into one binary glTF, each its own node.
///
/// One buffer, one mesh and one node per part. What a baked convex
/// decomposition needs: each piece has to stay a separate primitive,
/// because merging them gives back the concave solid the decomposition
/// exists to avoid — and because the importer reads one point set per
/// primitive.
pub fn to_glb_parts(parts: &[(&Mesh, &str)]) -> Result<Vec<u8>, ExportError> {
    if parts.is_empty() {
        return Err(ExportError::Empty);
    }
    for (mesh, _) in parts {
        validate(mesh)?;
    }

    let mut bin: Vec<u8> = Vec::new();
    let mut spans = Vec::with_capacity(parts.len());
    for (mesh, _) in parts {
        let vertex_bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
        let index_bytes: &[u8] = bytemuck::cast_slice(&mesh.indices);

        // Every view starts 4-byte aligned. `MeshVertex` is 32 bytes so
        // the vertex chunk already does, but padding keeps that from
        // being a silent assumption if the layout ever changes.
        bin.resize(align_to_four(bin.len()), 0);
        let vertex_offset = bin.len();
        bin.extend_from_slice(vertex_bytes);
        bin.resize(align_to_four(bin.len()), 0);
        let index_offset = bin.len();
        bin.extend_from_slice(index_bytes);

        spans.push(Span {
            vertex_offset,
            vertex_len: vertex_bytes.len(),
            index_offset,
            index_len: index_bytes.len(),
        });
    }
    let total = align_to_four(bin.len());
    bin.resize(total, 0);

    let root = build_root(parts, &spans, total);
    let json_chunk =
        json::serialize::to_string(&root).map_err(|e| ExportError::Json(e.to_string()))?;
    let glb = gltf::binary::Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            // `to_vec` recomputes this from the chunks, so the value here
            // is not load-bearing.
            length: 0,
        },
        json: Cow::Owned(json_chunk.into_bytes()),
        bin: Some(Cow::Owned(bin)),
    };
    glb.to_vec()
        .map_err(|e| ExportError::Container(e.to_string()))
}

/// Where one part's two views sit in the shared buffer.
struct Span {
    vertex_offset: usize,
    vertex_len: usize,
    index_offset: usize,
    index_len: usize,
}

/// Rejects meshes that would serialise into an invalid asset.
///
/// Checked here rather than left to the importer: an out-of-range index
/// in a written file is a crash in whatever opens it next, and the
/// generator that produced it is long out of the picture by then.
fn validate(mesh: &Mesh) -> Result<(), ExportError> {
    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
        return Err(ExportError::Empty);
    }
    if !mesh.indices.len().is_multiple_of(3) {
        return Err(ExportError::PartialTriangle(mesh.indices.len()));
    }
    let vertices = mesh.vertex_count();
    if let Some(&index) = mesh.indices.iter().find(|&&i| i >= vertices) {
        return Err(ExportError::IndexOutOfRange { index, vertices });
    }
    Ok(())
}

/// Builds the glTF JSON: one mesh and one node per part, over a shared
/// buffer.
fn build_root(parts: &[(&Mesh, &str)], spans: &[Span], buffer_len: usize) -> json::Root {
    let stride = mem::size_of::<MeshVertex>();

    let buffer = json::Buffer {
        byte_length: USize64::from(buffer_len),
        name: None,
        uri: None,
        extensions: None,
        extras: Default::default(),
    };

    let mut views = Vec::with_capacity(parts.len() * 2);
    let mut accessors = Vec::with_capacity(parts.len() * 4);
    let mut meshes = Vec::with_capacity(parts.len());
    let mut nodes = Vec::with_capacity(parts.len());

    for (part, ((mesh, name), span)) in parts.iter().zip(spans).enumerate() {
        views.push(json::buffer::View {
            buffer: json::Index::new(0),
            byte_length: USize64::from(span.vertex_len),
            byte_offset: Some(USize64::from(span.vertex_offset)),
            byte_stride: Some(json::buffer::Stride(stride)),
            name: None,
            target: None,
            extensions: None,
            extras: Default::default(),
        });
        views.push(json::buffer::View {
            buffer: json::Index::new(0),
            byte_length: USize64::from(span.index_len),
            byte_offset: Some(USize64::from(span.index_offset)),
            // Indices are tightly packed; a stride here is invalid glTF.
            byte_stride: None,
            name: None,
            target: None,
            extensions: None,
            extras: Default::default(),
        });

        let vertex_view = (part * 2) as u32;
        let index_view = vertex_view + 1;
        let count = USize64::from(mesh.vertices.len());

        // POSITION is the one accessor glTF requires min/max on — viewers
        // use it to frame the asset without reading the buffer.
        accessors.push(accessor(
            vertex_view,
            offset_of_position(),
            count,
            json::accessor::Type::Vec3,
            json::accessor::ComponentType::F32,
            Some(json::Value::from(mesh.aabb.min.to_array().to_vec())),
            Some(json::Value::from(mesh.aabb.max.to_array().to_vec())),
        ));
        accessors.push(accessor(
            vertex_view,
            offset_of_normal(),
            count,
            json::accessor::Type::Vec3,
            json::accessor::ComponentType::F32,
            None,
            None,
        ));
        accessors.push(accessor(
            vertex_view,
            offset_of_uv(),
            count,
            json::accessor::Type::Vec2,
            json::accessor::ComponentType::F32,
            None,
            None,
        ));
        accessors.push(accessor(
            index_view,
            0,
            USize64::from(mesh.indices.len()),
            json::accessor::Type::Scalar,
            json::accessor::ComponentType::U32,
            None,
            None,
        ));

        let base = (part * 4) as u32;
        meshes.push(json::Mesh {
            name: Some((*name).to_owned()),
            primitives: vec![json::mesh::Primitive {
                attributes: [
                    (
                        Valid(json::mesh::Semantic::Positions),
                        json::Index::new(base),
                    ),
                    (
                        Valid(json::mesh::Semantic::Normals),
                        json::Index::new(base + 1),
                    ),
                    (
                        Valid(json::mesh::Semantic::TexCoords(0)),
                        json::Index::new(base + 2),
                    ),
                ]
                .into_iter()
                .collect(),
                indices: Some(json::Index::new(base + 3)),
                material: None,
                mode: Valid(json::mesh::Mode::Triangles),
                targets: None,
                extensions: None,
                extras: Default::default(),
            }],
            weights: None,
            extensions: None,
            extras: Default::default(),
        });
        nodes.push(json::Node {
            mesh: Some(json::Index::new(part as u32)),
            name: Some((*name).to_owned()),
            ..Default::default()
        });
    }

    json::Root {
        asset: json::Asset {
            generator: Some(format!("kooch {}", env!("CARGO_PKG_VERSION"))),
            version: "2.0".into(),
            ..Default::default()
        },
        accessors,
        buffers: vec![buffer],
        buffer_views: views,
        meshes,
        scenes: vec![json::Scene {
            nodes: (0..nodes.len() as u32).map(json::Index::new).collect(),
            name: None,
            extensions: None,
            extras: Default::default(),
        }],
        nodes,
        scene: Some(json::Index::new(0)),
        ..Default::default()
    }
}

/// One accessor over a buffer view.
fn accessor(
    view: u32,
    byte_offset: usize,
    count: USize64,
    type_: json::accessor::Type,
    component_type: json::accessor::ComponentType,
    min: Option<json::Value>,
    max: Option<json::Value>,
) -> json::Accessor {
    json::Accessor {
        buffer_view: Some(json::Index::new(view)),
        byte_offset: Some(USize64::from(byte_offset)),
        count,
        component_type: Valid(json::accessor::GenericComponentType(component_type)),
        type_: Valid(type_),
        min,
        max,
        name: None,
        normalized: false,
        sparse: None,
        extensions: None,
        extras: Default::default(),
    }
}

// `MeshVertex` is `#[repr(C)]` with three float arrays, so the offsets are
// fixed by the layout. Spelled out rather than hard-coded so a change to
// the vertex layout moves the accessors with it.
const fn offset_of_position() -> usize {
    0
}
const fn offset_of_normal() -> usize {
    mem::size_of::<[f32; 3]>()
}
const fn offset_of_uv() -> usize {
    mem::size_of::<[f32; 3]>() * 2
}

/// Rounds up to the next multiple of four, as GLB chunks require.
fn align_to_four(n: usize) -> usize {
    (n + 3) & !3
}

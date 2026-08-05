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
    validate(mesh)?;

    let vertex_bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
    // The index chunk has to start 4-byte aligned; `MeshVertex` is 32
    // bytes so it already does, but padding here keeps that from being a
    // silent assumption if the layout ever changes.
    let index_offset = align_to_four(vertex_bytes.len());
    let index_bytes: &[u8] = bytemuck::cast_slice(&mesh.indices);

    let mut bin = Vec::with_capacity(index_offset + index_bytes.len());
    bin.extend_from_slice(vertex_bytes);
    bin.resize(index_offset, 0);
    bin.extend_from_slice(index_bytes);
    let total = align_to_four(bin.len());
    bin.resize(total, 0);

    let root = build_root(
        mesh,
        name,
        index_offset,
        vertex_bytes.len(),
        index_bytes.len(),
        total,
    );

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

/// Builds the glTF JSON for a single interleaved primitive.
fn build_root(
    mesh: &Mesh,
    name: &str,
    index_offset: usize,
    vertex_len: usize,
    index_len: usize,
    buffer_len: usize,
) -> json::Root {
    let stride = mem::size_of::<MeshVertex>();
    let count = USize64::from(mesh.vertices.len());

    let buffer = json::Buffer {
        byte_length: USize64::from(buffer_len),
        name: None,
        uri: None,
        extensions: None,
        extras: Default::default(),
    };

    let vertex_view = json::buffer::View {
        buffer: json::Index::new(0),
        byte_length: USize64::from(vertex_len),
        byte_offset: None,
        byte_stride: Some(json::buffer::Stride(stride)),
        name: None,
        target: None,
        extensions: None,
        extras: Default::default(),
    };
    let index_view = json::buffer::View {
        buffer: json::Index::new(0),
        byte_length: USize64::from(index_len),
        byte_offset: Some(USize64::from(index_offset)),
        // Indices are tightly packed; a stride here is invalid glTF.
        byte_stride: None,
        name: None,
        target: None,
        extensions: None,
        extras: Default::default(),
    };

    // POSITION is the one accessor glTF requires min/max on — viewers use
    // it to frame the asset without reading the buffer.
    let position = accessor(
        0,
        offset_of_position(),
        count,
        json::accessor::Type::Vec3,
        json::accessor::ComponentType::F32,
        Some(json::Value::from(mesh.aabb.min.to_array().to_vec())),
        Some(json::Value::from(mesh.aabb.max.to_array().to_vec())),
    );
    let normal = accessor(
        0,
        offset_of_normal(),
        count,
        json::accessor::Type::Vec3,
        json::accessor::ComponentType::F32,
        None,
        None,
    );
    let uv = accessor(
        0,
        offset_of_uv(),
        count,
        json::accessor::Type::Vec2,
        json::accessor::ComponentType::F32,
        None,
        None,
    );
    let indices = accessor(
        1,
        0,
        USize64::from(mesh.indices.len()),
        json::accessor::Type::Scalar,
        json::accessor::ComponentType::U32,
        None,
        None,
    );

    let primitive = json::mesh::Primitive {
        attributes: [
            (Valid(json::mesh::Semantic::Positions), json::Index::new(0)),
            (Valid(json::mesh::Semantic::Normals), json::Index::new(1)),
            (
                Valid(json::mesh::Semantic::TexCoords(0)),
                json::Index::new(2),
            ),
        ]
        .into_iter()
        .collect(),
        indices: Some(json::Index::new(3)),
        material: None,
        mode: Valid(json::mesh::Mode::Triangles),
        targets: None,
        extensions: None,
        extras: Default::default(),
    };

    json::Root {
        asset: json::Asset {
            generator: Some(format!("kooch {}", env!("CARGO_PKG_VERSION"))),
            version: "2.0".into(),
            ..Default::default()
        },
        accessors: vec![position, normal, uv, indices],
        buffers: vec![buffer],
        buffer_views: vec![vertex_view, index_view],
        meshes: vec![json::Mesh {
            name: Some(name.to_owned()),
            primitives: vec![primitive],
            weights: None,
            extensions: None,
            extras: Default::default(),
        }],
        nodes: vec![json::Node {
            mesh: Some(json::Index::new(0)),
            name: Some(name.to_owned()),
            ..Default::default()
        }],
        scenes: vec![json::Scene {
            nodes: vec![json::Index::new(0)],
            name: None,
            extensions: None,
            extras: Default::default(),
        }],
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

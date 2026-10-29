//! Mesh assets shared by the meshlet GPU-driven pipeline.

mod asset;
pub mod export;
mod gltf_loader;
pub mod primitives;
mod vertex;

pub use asset::Mesh;
pub use export::{ExportError, SimplifyTarget, simplify, to_glb, to_glb_parts};
pub use gltf_loader::{
    GltfMeshError, GltfMeshLoader, SidecarReader, parse_mesh_bytes, parse_mesh_bytes_full,
    parse_mesh_bytes_with_scale, parse_mesh_parts, parse_mesh_read, parse_parts_read, siblings,
};
pub use primitives::Primitive;
pub use vertex::{Aabb, MeshVertex};

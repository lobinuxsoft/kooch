//! Mesh assets shared by the meshlet GPU-driven pipeline.
//!
//! The actual rendering lives in [`crate::meshlet`]: meshlet generation
//! consumes a [`Mesh`] (vertices + indices + AABB), packs it into
//! `MeshletMesh`, and the `MeshletRenderStage` runs cull + visibility
//! raster + deferred shading off it.
//!
//! This module owns:
//! - [`Mesh`] — CPU-side POD asset stored in `Assets<Mesh>`.
//! - [`GltfMeshLoader`] — `AssetLoader<Mesh>` impl for glTF / GLB.
//! - [`MeshVertex`] / [`Aabb`] — shared vertex layout + local AABB.
//! - [`Primitive`] — procedurally generated cube / sphere / capsule / …

mod asset;
pub mod export;
mod gltf_loader;
pub mod primitives;
mod vertex;

pub use asset::Mesh;
pub use export::{ExportError, SimplifyTarget, simplify, to_glb, to_glb_parts};
pub use gltf_loader::{
    GltfMeshError, GltfMeshLoader, parse_mesh_bytes, parse_mesh_bytes_full,
    parse_mesh_bytes_with_scale, parse_mesh_parts,
};
pub use primitives::Primitive;
pub use vertex::{Aabb, MeshVertex};

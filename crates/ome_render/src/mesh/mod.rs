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

mod asset;
mod gltf_loader;
mod vertex;

pub use asset::Mesh;
pub use gltf_loader::{GltfMeshError, GltfMeshLoader, parse_mesh_bytes};
pub use vertex::{Aabb, MeshVertex};

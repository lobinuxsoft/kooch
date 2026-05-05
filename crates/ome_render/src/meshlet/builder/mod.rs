//! `Mesh` → `MeshletMesh` offline builder using `meshopt` + METIS.
//!
//! Two production entry points:
//!
//! - [`build_meshlets_from_mesh`] — single-LOD output (the legacy /
//!   default path). Every meshlet is a DAG root with `lod_error = 0`.
//! - [`build_meshlets_lod_chain`] — runs `meshopt::simplify_with_locks`
//!   per group to produce a chain of LODs and concatenates them into
//!   one [`MeshletMesh`]. Meshlet groups are computed with METIS k-way
//!   graph partitioning over the shared-vertex connectivity graph
//!   (Karis SIGGRAPH 2021, confirmed by Scthe/nanite-webgpu and
//!   pettett/multires); cell-boundary vertices are explicit-locked
//!   during simplify (Ponchio §3.4.3).
//!
//! The module is split by concern: [`error`] holds the error type,
//! [`lod_config`] the chain knobs, [`common`] shared helpers used by
//! both entry points, [`single_lod`] the single-LOD path,
//! [`lod_chain`] the multi-LOD chain build, [`grouping`] the METIS
//! partitioner + boundary detection.

mod common;
mod error;
mod grouping;
mod lod_chain;
mod lod_config;
mod single_lod;

#[cfg(test)]
mod test_support;

pub use error::MeshletBuildError;
pub use lod_chain::build_meshlets_lod_chain;
pub use lod_config::LodConfig;
pub use single_lod::{build_default_meshlets, build_meshlets_from_mesh};

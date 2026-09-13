//! `Mesh` → `MeshletMesh` offline builder using `meshopt` + METIS.

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

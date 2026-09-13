//! `AssetLoader<MeshletMesh>` — parses `.glb` / `.gltf` bytes into a GPU-ready meshlet asset.

use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};

use crate::mesh::parse_mesh_bytes_full;

use super::asset::{DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES, MeshletMesh};
use super::builder::{LodConfig, MeshletBuildError, build_meshlets_lod_chain};

/// Loads `.glb` / `.gltf` files directly into [`MeshletMesh`].
#[derive(Debug, Default, Clone, Copy)]
pub struct MeshletMeshLoader;

impl AssetLoader<MeshletMesh> for MeshletMeshLoader {
    fn extensions(&self) -> &[&'static str] {
        &["glb", "gltf"]
    }

    fn load(&self, bytes: &[u8], ctx: &mut LoadContext<'_>) -> AssetResult<MeshletMesh> {
        let mesh = parse_mesh_bytes_full(bytes, 1.0, ctx.path.parent())
            .map_err(|e| AssetError::Loader(Box::new(e)))?;
        build_meshlets_lod_chain(
            &mesh,
            DEFAULT_MAX_VERTICES,
            DEFAULT_MAX_TRIANGLES,
            0.5,
            LodConfig::default(),
        )
        .map_err(|e: MeshletBuildError| AssetError::Loader(Box::new(e)))
    }
}

#[cfg(test)]
mod tests;

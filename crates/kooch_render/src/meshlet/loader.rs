//! `AssetLoader<MeshletMesh>` — parses `.glb` / `.gltf` bytes into a
//! GPU-ready meshlet asset.
//!
//! Pipeline: bytes → [`parse_mesh_bytes`] (CPU `Mesh`) →
//! [`build_meshlets_lod_chain`] (Nanite-grouped DAG with group ids)
//! → [`MeshletMesh`]. Asset bytes never touch disk twice — the
//! `AssetServer` reads them once and the loader builds the meshlet
//! representation in-process.
//!
//! # LOD chain by default (post-#465)
//!
//! `build_meshlets_lod_chain` produces a per-LOD-level grouping with
//! parent-child links AND group identifiers (group_index /
//! children_group_index). The runtime 2-pass cull
//! ([`super::dispatcher::MeshletCull::dispatch_scene_pool_atomic`])
//! consumes those ids: pass 1 atomicMaxes pixel error per group,
//! pass 2 selects atomically per group. Sibling meshlets that share
//! a group descend together or stay together — no torn coverage
//! seam, no flicker between LOD transitions.
//!
//! Per-asset overrides (disable LOD chain, custom `LodConfig`) land
//! alongside the `.meta`-driven import settings system (Plan B
//! part 2). The minimal triangle fixture used by the tests collapses
//! naturally to a single-level chain because `meshopt::simplify`
//! cannot reduce a single triangle further.
//!
//! Loader extensions match `GltfMeshLoader` (`glb`, `gltf`) so the
//! same source files can serve both `Assets<Mesh>` (raw geometry,
//! useful for tooling / extracting BVHs / etc.) and
//! `Assets<MeshletMesh>` (the rendering form). Each `Assets<T>` has
//! its own GUID cache, so the same `.glb` produces two distinct
//! handles — one per type.

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

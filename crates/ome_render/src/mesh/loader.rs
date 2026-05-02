//! Legacy path-based mesh cache (`MeshLoader`).
//!
//! Wraps the [`GltfMeshLoader`](super::GltfMeshLoader) parser + the
//! upload helper on [`Mesh`](super::Mesh) so the existing
//! [`MeshPassRenderer`](super::MeshPassRenderer) keeps working unchanged
//! while the asset pipeline migrates to `AssetServer + Assets<Mesh>`.
//!
//! NEW code SHOULD use:
//! ```ignore
//! let mut server = AssetServer::new();
//! server.register_loader::<Mesh, _>(GltfMeshLoader);
//! resources.insert(Assets::<Mesh>::new());
//! let handle = server.load::<Mesh>("models/cube.glb", &mut resources)?;
//! ```
//!
//! `MeshLoader` itself is scheduled for removal once the renderer flips
//! to the new flow (follow-up issue).

use std::collections::HashMap;
use std::sync::Arc;

use super::asset::Mesh;
use super::gltf_loader::{GltfMeshError, parse_mesh_bytes};
use super::gpu_mesh::GpuMesh;

/// Errors produced while loading a glTF asset into a [`GpuMesh`] via the
/// legacy path-based cache.
#[derive(Debug)]
pub enum MeshLoadError {
    /// File could not be read from disk.
    Io(std::io::Error),
    /// Underlying parser error from the glTF asset loader.
    Parse(GltfMeshError),
}

impl std::fmt::Display for MeshLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "mesh load I/O failed: {e}"),
            Self::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MeshLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Parse(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for MeshLoadError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<GltfMeshError> for MeshLoadError {
    fn from(e: GltfMeshError) -> Self {
        Self::Parse(e)
    }
}

/// Loads glTF / GLB files into [`GpuMesh`] handles, caching results by path.
///
/// The cache holds `Arc<GpuMesh>` so the same asset shared by many entities
/// uses one set of GPU buffers. Cache invalidation (hot-reload) is out of
/// scope for the MVP.
#[derive(Default)]
pub struct MeshLoader {
    cache: HashMap<String, Arc<GpuMesh>>,
}

impl MeshLoader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the cached mesh for `path`, or loads + caches it on demand.
    ///
    /// Internally: read bytes → `parse_mesh_bytes` → [`Mesh::upload`] →
    /// `Arc<GpuMesh>` cache insert.
    pub fn get_or_load(
        &mut self,
        device: &wgpu::Device,
        path: &str,
    ) -> Result<Arc<GpuMesh>, MeshLoadError> {
        if let Some(cached) = self.cache.get(path) {
            return Ok(cached.clone());
        }
        let bytes = std::fs::read(path)?;
        let mesh: Mesh = parse_mesh_bytes(&bytes)?;
        let gpu = mesh.upload(device);
        let arc = Arc::new(gpu);
        self.cache.insert(path.to_string(), arc.clone());
        tracing::info!(
            path,
            vertices = arc.vertex_count,
            indices = arc.index_count,
            "loaded glTF mesh",
        );
        Ok(arc)
    }

    /// Number of meshes currently cached. Useful for telemetry and tests.
    pub fn cached_count(&self) -> usize {
        self.cache.len()
    }
}

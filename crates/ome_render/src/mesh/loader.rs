//! glTF 2.0 mesh loader with an in-memory cache keyed by file path.
//!
//! MVP scope (issue #129): loads the **first primitive** of the **first
//! mesh** in the document. Materials, scenes, node hierarchy, skinning,
//! morph targets and multi-primitive meshes are out of scope and ignored.

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use wgpu::util::DeviceExt;

use super::gpu_mesh::{Aabb, GpuMesh, MeshVertex};

/// Errors produced while loading a glTF asset into a [`GpuMesh`].
#[derive(Debug)]
pub enum MeshLoadError {
    /// The `gltf` crate failed to parse or import the file.
    Gltf(gltf::Error),
    /// Required vertex attribute was missing from the primitive.
    MissingAttribute(&'static str),
    /// The document contained no meshes (or no primitives).
    EmptyDocument,
}

impl std::fmt::Display for MeshLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gltf(e) => write!(f, "gltf import failed: {e}"),
            Self::MissingAttribute(name) => write!(f, "primitive missing required attribute: {name}"),
            Self::EmptyDocument => write!(f, "gltf document contains no mesh primitives"),
        }
    }
}

impl std::error::Error for MeshLoadError {}

impl From<gltf::Error> for MeshLoadError {
    fn from(value: gltf::Error) -> Self {
        Self::Gltf(value)
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

    /// Returns the cached mesh for `path`, or loads and caches it on demand.
    pub fn get_or_load(
        &mut self,
        device: &wgpu::Device,
        path: &str,
    ) -> Result<Arc<GpuMesh>, MeshLoadError> {
        if let Some(cached) = self.cache.get(path) {
            return Ok(cached.clone());
        }
        let mesh = load_from_disk(device, path)?;
        let arc = Arc::new(mesh);
        self.cache.insert(path.to_string(), arc.clone());
        tracing::info!(path, vertices = arc.vertex_count, indices = arc.index_count, "loaded glTF mesh");
        Ok(arc)
    }

    /// Number of meshes currently cached. Useful for telemetry and tests.
    pub fn cached_count(&self) -> usize {
        self.cache.len()
    }
}

fn load_from_disk(device: &wgpu::Device, path: &str) -> Result<GpuMesh, MeshLoadError> {
    let (document, buffers, _images) = gltf::import(path)?;

    let primitive = document
        .meshes()
        .next()
        .and_then(|m| m.primitives().next())
        .ok_or(MeshLoadError::EmptyDocument)?;

    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or(MeshLoadError::MissingAttribute("POSITION"))?
        .collect();

    let vertex_count = positions.len();

    let normals: Vec<[f32; 3]> = reader
        .read_normals()
        .map(|iter| iter.collect())
        .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; vertex_count]);

    let uvs: Vec<[f32; 2]> = reader
        .read_tex_coords(0)
        .map(|coords| coords.into_f32().collect())
        .unwrap_or_else(|| vec![[0.0, 0.0]; vertex_count]);

    let mut aabb = Aabb::empty();
    let vertices: Vec<MeshVertex> = (0..vertex_count)
        .map(|i| {
            let p = positions[i];
            aabb.expand(Vec3::from_array(p));
            MeshVertex {
                position: p,
                normal: normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]),
                uv: uvs.get(i).copied().unwrap_or([0.0, 0.0]),
            }
        })
        .collect();

    let indices: Vec<u32> = match reader.read_indices() {
        Some(idx) => idx.into_u32().collect(),
        None => (0..vertex_count as u32).collect(),
    };

    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mesh_vertex_buffer"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mesh_index_buffer"),
        contents: bytemuck::cast_slice(&indices),
        usage: wgpu::BufferUsages::INDEX,
    });

    Ok(GpuMesh {
        vertex_buffer,
        index_buffer,
        vertex_count: vertex_count as u32,
        index_count: indices.len() as u32,
        index_format: wgpu::IndexFormat::Uint32,
        aabb,
    })
}

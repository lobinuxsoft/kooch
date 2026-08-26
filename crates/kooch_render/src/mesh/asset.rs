//! `Mesh` — CPU-side asset type loaded from glTF.
//!
//! A `Mesh` holds geometry data parsed from disk. It is the type
//! `GltfMeshLoader` produces and the type [`Assets<Mesh>`] stores. The
//! meshlet builder consumes it directly to produce GPU-resident
//! `MeshletMesh` data — there is no separate raw-mesh GPU upload.

use glam::Vec3;

use super::vertex::{Aabb, MeshVertex};

/// CPU-side mesh data: interleaved vertex array + 32-bit indices + AABB.
///
/// Layout matches [`MeshVertex`] for one-shot upload via
/// `bytemuck::cast_slice`. Loaders are responsible for filling missing
/// attributes (normals, uvs) with sensible defaults so the GPU layout stays
/// consistent across assets.
#[derive(Debug, Clone)]
pub struct Mesh {
    /// Interleaved vertex stream (position + normal + uv).
    pub vertices: Vec<MeshVertex>,
    /// Index list (always `u32` for simplicity — 16-bit upcast at parse time).
    pub indices: Vec<u32>,
    /// Local-space bounds, computed from positions during load.
    pub aabb: Aabb,
}

impl Mesh {
    /// Empty mesh (no vertices, no indices). Useful as a placeholder while
    /// async loads complete.
    pub fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            aabb: Aabb::empty(),
        }
    }

    /// Builds a mesh from raw streams. Computes AABB from positions.
    pub fn from_arrays(vertices: Vec<MeshVertex>, indices: Vec<u32>) -> Self {
        let mut aabb = Aabb::empty();
        for v in &vertices {
            aabb.expand(Vec3::from_array(v.position));
        }
        Self {
            vertices,
            indices,
            aabb,
        }
    }

    /// Vertex count.
    pub fn vertex_count(&self) -> u32 {
        self.vertices.len() as u32
    }

    /// Index count.
    pub fn index_count(&self) -> u32 {
        self.indices.len() as u32
    }
}

impl Default for Mesh {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests;

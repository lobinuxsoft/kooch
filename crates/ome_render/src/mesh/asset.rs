//! `Mesh` — CPU-side asset type loaded from glTF.
//!
//! A `Mesh` holds geometry data parsed from disk. It is the type
//! `GltfMeshLoader` produces and the type [`Assets<Mesh>`] stores. GPU
//! upload happens separately via [`Mesh::upload`], yielding a [`GpuMesh`]
//! the render pass consumes.
//!
//! Splitting CPU `Mesh` from GPU [`GpuMesh`] lets the same asset be:
//! - Loaded once, uploaded once
//! - Inspected by tools (mesh viewer, exporter, baker) without GPU context
//! - Re-uploaded after edits without re-parsing the source file

use glam::Vec3;
use wgpu::util::DeviceExt;

use super::gpu_mesh::{Aabb, GpuMesh, MeshVertex};

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

    /// Uploads `self` into GPU buffers, returning a [`GpuMesh`] ready for
    /// draw calls. Allocates two `wgpu::Buffer`s: one VERTEX, one INDEX.
    pub fn upload(&self, device: &wgpu::Device) -> GpuMesh {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh_vertex_buffer"),
            contents: bytemuck::cast_slice(&self.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh_index_buffer"),
            contents: bytemuck::cast_slice(&self.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        GpuMesh {
            vertex_buffer,
            index_buffer,
            vertex_count: self.vertex_count(),
            index_count: self.index_count(),
            index_format: wgpu::IndexFormat::Uint32,
            aabb: self.aabb,
        }
    }
}

impl Default for Mesh {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(p: [f32; 3]) -> MeshVertex {
        MeshVertex {
            position: p,
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
        }
    }

    #[test]
    fn empty_mesh_has_zero_counts() {
        let m = Mesh::empty();
        assert_eq!(m.vertex_count(), 0);
        assert_eq!(m.index_count(), 0);
        assert!(m.aabb.is_empty());
    }

    #[test]
    fn from_arrays_computes_aabb_from_positions() {
        let verts = vec![
            vertex([0.0, 0.0, 0.0]),
            vertex([2.0, 4.0, -1.0]),
            vertex([-3.0, 1.0, 5.0]),
        ];
        let idx = vec![0, 1, 2];
        let mesh = Mesh::from_arrays(verts, idx);

        assert_eq!(mesh.vertex_count(), 3);
        assert_eq!(mesh.index_count(), 3);
        assert_eq!(mesh.aabb.min, Vec3::new(-3.0, 0.0, -1.0));
        assert_eq!(mesh.aabb.max, Vec3::new(2.0, 4.0, 5.0));
    }

    #[test]
    fn from_arrays_with_no_vertices_produces_empty_aabb() {
        let mesh = Mesh::from_arrays(Vec::new(), Vec::new());
        assert!(mesh.aabb.is_empty());
    }
}

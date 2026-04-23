//! GPU-resident mesh data and vertex layout.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// Vertex layout uploaded to GPU. Stride is 32 bytes, attributes are
/// position (location 0), normal (location 1), uv0 (location 2).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// Axis-aligned bounding box in mesh-local space. Used today for debug
/// metrics; future culling and broadphase will read it.
#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// Returns an empty AABB (min = +∞, max = -∞). Use [`Aabb::expand`]
    /// to grow it from a stream of points.
    pub fn empty() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }

    /// Grows the AABB to include `point`.
    pub fn expand(&mut self, point: Vec3) {
        self.min = self.min.min(point);
        self.max = self.max.max(point);
    }

    /// Returns `true` when no points have been added yet.
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x
    }
}

impl Default for Aabb {
    fn default() -> Self {
        Self::empty()
    }
}

/// GPU-resident mesh: vertex buffer, index buffer, vertex/index counts,
/// and a local-space AABB. Created by [`super::MeshLoader`].
pub struct GpuMesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub vertex_count: u32,
    pub index_count: u32,
    pub index_format: wgpu::IndexFormat,
    pub aabb: Aabb,
}

/// Vertex buffer layout descriptor used by the mesh pipeline.
pub(super) const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 12,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 24,
        shader_location: 2,
    },
];

pub(super) fn vertex_buffer_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<MeshVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &VERTEX_ATTRIBUTES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_size_matches_layout_stride() {
        assert_eq!(std::mem::size_of::<MeshVertex>(), 32);
        assert_eq!(vertex_buffer_layout().array_stride, 32);
    }

    #[test]
    fn aabb_starts_empty_and_expands() {
        let mut aabb = Aabb::empty();
        assert!(aabb.is_empty());
        aabb.expand(Vec3::new(1.0, 2.0, 3.0));
        aabb.expand(Vec3::new(-1.0, 5.0, 0.0));
        assert!(!aabb.is_empty());
        assert_eq!(aabb.min, Vec3::new(-1.0, 2.0, 0.0));
        assert_eq!(aabb.max, Vec3::new(1.0, 5.0, 3.0));
    }
}

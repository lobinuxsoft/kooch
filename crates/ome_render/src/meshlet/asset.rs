//! `MeshletMesh` CPU asset + per-meshlet descriptor.
//!
//! Mirrors the data layout the GPU compute culling shader will consume
//! once Phase 1.D lands the upload + culling passes. Public layout is
//! intentionally GPU-friendly (POD, fixed-size descriptors, Vec<u32>
//! index lists) so future `bytemuck::cast_slice` upload is one-liner.

use bytemuck::{Pod, Zeroable};

use crate::mesh::{Aabb, MeshVertex};

/// Recommended max vertices per meshlet. 64 lines up with NV's mesh
/// shader sweet spot and is what Bevy 0.16 / UE5 Nanite use as default.
pub const DEFAULT_MAX_VERTICES: usize = 64;

/// Recommended max triangles per meshlet. 124 is the largest multiple
/// of 4 ≤ 128 (`meshopt` requires `max_triangles` divisible by 4).
pub const DEFAULT_MAX_TRIANGLES: usize = 124;

/// Per-meshlet metadata. POD, repr(C), 80 bytes — packs into a uniform
/// storage buffer for compute culling without a single `if let` on
/// upload. Layout fields:
///
/// - `vertex_offset` / `triangle_offset` index into the parent
///   [`MeshletMesh::meshlet_vertices`] / `meshlet_triangles` arrays.
/// - `vertex_count` ≤ [`DEFAULT_MAX_VERTICES`], `triangle_count` ≤
///   [`DEFAULT_MAX_TRIANGLES`].
/// - `aabb_min` / `aabb_max` are world-local. The compute shader
///   transforms them into world space per-instance.
/// - `cone_apex` / `cone_axis` / `cone_cutoff` form a normal cone
///   for backface culling: if the camera is in the half-space
///   defined by the cone, the meshlet is fully back-facing and can
///   skip both rasterization and visibility-buffer emission.
/// - `bounding_radius` covers all of the meshlet's vertices from
///   `cone_apex` — used by frustum and occlusion (Hi-Z) culling.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MeshletDescriptor {
    pub vertex_offset: u32,
    pub triangle_offset: u32,
    pub vertex_count: u32,
    pub triangle_count: u32,
    pub aabb_min: [f32; 3],
    pub _pad0: u32,
    pub aabb_max: [f32; 3],
    pub _pad1: u32,
    pub cone_apex: [f32; 3],
    pub bounding_radius: f32,
    pub cone_axis: [f32; 3],
    pub cone_cutoff: f32,
}

impl MeshletDescriptor {
    /// Bytes used by one descriptor — kept as `const` so future
    /// `wgpu::BindingType::Buffer` declarations don't need to redo the
    /// computation.
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

/// CPU-side meshlet representation.
///
/// Storage layout is the same as Bevy's `MeshletMesh` (and Nanite's
/// "MeshletData" struct): one big vertex pool + flat index arrays
/// referencing it + per-meshlet descriptors.
#[derive(Debug, Clone)]
pub struct MeshletMesh {
    /// Shared vertex pool. Each meshlet's `vertex_offset` slice contains
    /// indices INTO this pool.
    pub vertices: Vec<MeshVertex>,
    /// Per-meshlet vertex indices. `vertex_offset .. vertex_offset+vertex_count`
    /// of this array gives the meshlet's vertex set.
    pub meshlet_vertices: Vec<u32>,
    /// Per-meshlet triangle data: 3 bytes per triangle (one byte per
    /// corner index into the meshlet's vertex set, NOT into `vertices`).
    /// `meshopt` packs them this way for compactness — the GPU shader
    /// reconstructs full vertex indices via `meshlet_vertices[base + idx]`.
    pub meshlet_triangles: Vec<u8>,
    /// Per-meshlet metadata. Length = number of meshlets.
    pub meshlets: Vec<MeshletDescriptor>,
    /// Bounds of the entire mesh.
    pub aabb: Aabb,
}

impl MeshletMesh {
    /// Number of meshlets.
    pub fn meshlet_count(&self) -> u32 {
        self.meshlets.len() as u32
    }

    /// Total vertex count in the pool.
    pub fn total_vertex_count(&self) -> u32 {
        self.vertices.len() as u32
    }

    /// Total triangle count summed across every meshlet.
    pub fn total_triangle_count(&self) -> u32 {
        self.meshlets
            .iter()
            .map(|m| m.triangle_count)
            .sum::<u32>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_layout_is_repr_c_pod() {
        // Fixed size lets the GPU upload assume a stride.
        assert_eq!(MeshletDescriptor::SIZE, 80);
    }

    #[test]
    fn descriptor_constructs_via_zeroable() {
        let d = MeshletDescriptor::zeroed();
        assert_eq!(d.vertex_count, 0);
        assert_eq!(d.triangle_count, 0);
    }

    #[test]
    fn empty_mesh_reports_zero_counts() {
        let mesh = MeshletMesh {
            vertices: Vec::new(),
            meshlet_vertices: Vec::new(),
            meshlet_triangles: Vec::new(),
            meshlets: Vec::new(),
            aabb: Aabb::empty(),
        };
        assert_eq!(mesh.meshlet_count(), 0);
        assert_eq!(mesh.total_vertex_count(), 0);
        assert_eq!(mesh.total_triangle_count(), 0);
    }
}

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

/// Per-meshlet metadata. POD, repr(C), 96 bytes — packs into a storage
/// buffer for compute culling without a single `if let` on upload.
///
/// Layout (offsets in bytes):
/// ```text
///  0  vertex_offset (u32)        index into MeshletMesh::meshlet_vertices
///  4  triangle_offset (u32)      byte offset into MeshletMesh::meshlet_triangles
///  8  vertex_count (u32)         ≤ DEFAULT_MAX_VERTICES
/// 12  triangle_count (u32)       ≤ DEFAULT_MAX_TRIANGLES
/// 16  aabb_min ([f32;3])         local-space, render pass transforms per-instance
/// 28  _pad0 (u32)
/// 32  aabb_max ([f32;3])
/// 44  _pad1 (u32)
/// 48  bounds_center ([f32;3])    bounding sphere centre (frustum + Hi-Z cull)
/// 60  bounding_radius (f32)
/// 64  cone_apex ([f32;3])        normal-cone apex (backface cull)
/// 76  cone_cutoff (f32)          cosine of half-angle
/// 80  cone_axis ([f32;3])        normalized cone axis
/// 92  _pad2 (u32)
/// ```
///
/// `bounds_center` and `cone_apex` are deliberately separate: meshopt
/// returns them as distinct vectors and the cone test
/// `dot(normalize(camera - cone_apex), cone_axis) >= cone_cutoff` is
/// only correct against the real apex, not the sphere centre. The
/// frustum cull keeps using `bounds_center` + `bounding_radius` as
/// before.
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
    pub bounds_center: [f32; 3],
    pub bounding_radius: f32,
    pub cone_apex: [f32; 3],
    pub cone_cutoff: f32,
    pub cone_axis: [f32; 3],
    pub _pad2: u32,
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
        // Fixed size lets the GPU upload assume a stride. Bumped from
        // 80B to 96B in PR-5b to fit a real cone_apex separate from the
        // bounding-sphere centre.
        assert_eq!(MeshletDescriptor::SIZE, 96);
    }

    #[test]
    fn descriptor_field_offsets_match_shader_layout() {
        use std::mem::offset_of;
        // Drift here would break meshlet_cull.wgsl / meshlet_main.wgsl
        // bind reads silently. The asserts mirror the WGSL `struct
        // MeshletDescriptor` declaration.
        assert_eq!(offset_of!(MeshletDescriptor, vertex_offset), 0);
        assert_eq!(offset_of!(MeshletDescriptor, triangle_offset), 4);
        assert_eq!(offset_of!(MeshletDescriptor, vertex_count), 8);
        assert_eq!(offset_of!(MeshletDescriptor, triangle_count), 12);
        assert_eq!(offset_of!(MeshletDescriptor, aabb_min), 16);
        assert_eq!(offset_of!(MeshletDescriptor, aabb_max), 32);
        assert_eq!(offset_of!(MeshletDescriptor, bounds_center), 48);
        assert_eq!(offset_of!(MeshletDescriptor, bounding_radius), 60);
        assert_eq!(offset_of!(MeshletDescriptor, cone_apex), 64);
        assert_eq!(offset_of!(MeshletDescriptor, cone_cutoff), 76);
        assert_eq!(offset_of!(MeshletDescriptor, cone_axis), 80);
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

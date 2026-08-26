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

/// Sentinel value for [`MeshletDescriptor::parent_meshlet_index`] —
/// the meshlet has no parent (root of the LOD DAG, coarsest level).
/// The runtime LOD selector treats this as the descent stopping
/// point.
pub const MESHLET_ROOT_PARENT: u32 = u32::MAX;

/// Sentinel value for [`MeshletDescriptor::group_index`] /
/// [`MeshletDescriptor::children_group_index`] — the meshlet does
/// not belong to a group on this side. Used for:
/// - `group_index = MESHLET_GROUP_NONE` ⇒ root meshlet, no parent
///   group above (the descent test "above_too_coarse" passes
///   trivially).
/// - `children_group_index = MESHLET_GROUP_NONE` ⇒ LOD 0 meshlet,
///   no children below (the "below_fine" test passes trivially).
pub const MESHLET_GROUP_NONE: u32 = u32::MAX;

/// Per-meshlet metadata. POD, repr(C), 112 bytes — packs into a
/// storage buffer for compute culling without a single `if let` on
/// upload.
///
/// Layout (offsets in bytes):
/// ```text
///   0  vertex_offset (u32)              index into MeshletMesh::meshlet_vertices
///   4  triangle_offset (u32)            byte offset into MeshletMesh::meshlet_triangles
///   8  vertex_count (u32)               ≤ DEFAULT_MAX_VERTICES
///  12  triangle_count (u32)             ≤ DEFAULT_MAX_TRIANGLES
///  16  aabb_min ([f32;3])               local-space, render pass transforms per-instance
///  28  parent_meshlet_index (u32)       index into the same MeshletMesh::meshlets
///                                       array; MESHLET_ROOT_PARENT for roots
///  32  aabb_max ([f32;3])
///  44  lod_error (f32)                  meshopt::simplify error this meshlet
///                                       represents (0.0 for LOD 0 / unsimplified)
///  48  bounds_center ([f32;3])          bounding sphere centre (frustum + Hi-Z cull)
///  60  bounding_radius (f32)
///  64  cone_apex ([f32;3])              normal-cone apex (backface cull)
///  76  cone_cutoff (f32)                cosine of half-angle
///  80  cone_axis ([f32;3])              normalized cone axis
///  92  group_index (u32)                #465 — id of the group this meshlet
///                                       is a child of (parents share the
///                                       group). MESHLET_GROUP_NONE for roots.
///  96  children_group_index (u32)       #465 — id of the group this meshlet
///                                       is a parent of (siblings share it).
///                                       MESHLET_GROUP_NONE for LOD 0 meshlets.
/// 100  lod_level (u32)              #467 — chain depth, 0 = LOD 0,
///                                      incremented per simplification step
/// 104  _pad4 (u32)
/// 108  _pad5 (u32)
/// ```
///
/// `bounds_center` and `cone_apex` are deliberately separate: meshopt
/// returns them as distinct vectors and the cone test
/// `dot(normalize(camera - cone_apex), cone_axis) >= cone_cutoff` is
/// only correct against the real apex, not the sphere centre. The
/// frustum cull keeps using `bounds_center` + `bounding_radius` as
/// before.
///
/// `parent_meshlet_index` and `lod_error` form the DAG used by the
/// continuous-LOD selector (#442). Single-LOD meshes leave both at
/// their sentinels (parent = MESHLET_ROOT_PARENT, error = 0.0); the
/// selector treats that as "always pick this meshlet" so the legacy
/// path keeps working bit-identically.
///
/// `group_index` / `children_group_index` drive the 2-pass cull's
/// group-atomic descent (#465): pass 1 atomicMaxes pixel error per
/// group, pass 2 selects atomically based on the group's max so
/// sibling parents emitted by the same group can never split between
/// "descend" and "stay" decisions. Single-LOD assets keep both at
/// MESHLET_GROUP_NONE.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MeshletDescriptor {
    pub vertex_offset: u32,
    pub triangle_offset: u32,
    pub vertex_count: u32,
    pub triangle_count: u32,
    pub aabb_min: [f32; 3],
    /// Index of the parent meshlet in the same
    /// [`MeshletMesh::meshlets`] array; [`MESHLET_ROOT_PARENT`] for
    /// roots at the coarsest LOD level.
    pub parent_meshlet_index: u32,
    pub aabb_max: [f32; 3],
    /// `meshopt::simplify` error this meshlet represents in mesh
    /// units. `0.0` for LOD 0 (unsimplified).
    pub lod_error: f32,
    pub bounds_center: [f32; 3],
    pub bounding_radius: f32,
    pub cone_apex: [f32; 3],
    pub cone_cutoff: f32,
    pub cone_axis: [f32; 3],
    /// Id of the group this meshlet belongs to as a CHILD. Siblings
    /// in the same group share this id. Used in the 2-pass cull
    /// (#465): pass 1 atomicMaxes the meshlet's parent's pixel
    /// error into `group_max_err[group_index]`; pass 2 reads the
    /// same slot to decide whether the group's parents are too
    /// coarse for this distance. [`MESHLET_GROUP_NONE`] for roots
    /// (no group above).
    pub group_index: u32,
    /// Id of the group this meshlet belongs to as a PARENT. Sibling
    /// parents emitted by the same group share this id. Pass 2 of
    /// the 2-pass cull reads `group_max_err[children_group_index]`
    /// to decide whether *this* meshlet's level is fine (versus
    /// descending further into the children). [`MESHLET_GROUP_NONE`]
    /// for LOD 0 meshlets (no group below).
    pub children_group_index: u32,
    /// Chain depth: 0 = LOD 0 (full detail), 1 = parents from the
    /// first per-group simplification, etc. Drives the LOD-stack
    /// debug inspector (#467) and `MeshInstance.lod_force_level`
    /// filtering in the cull shader.
    pub lod_level: u32,
    pub _pad4: u32,
    pub _pad5: u32,
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
        self.meshlets.iter().map(|m| m.triangle_count).sum::<u32>()
    }
}

#[cfg(test)]
mod tests;

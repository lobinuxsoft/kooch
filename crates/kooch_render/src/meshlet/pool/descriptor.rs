//! Per-mesh descriptor + handle types for the [`GlobalMeshPool`](super::GlobalMeshPool).

use bytemuck::{Pod, Zeroable};

/// Per-mesh metadata living in the global pool. `inst.mesh_id` is an
/// index into this array; the cull / vbuf / deferred shaders read it
/// to find a mesh's slice of the concatenated arrays.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MeshDescriptor {
    pub first_meshlet: u32,
    pub meshlet_count: u32,
    /// Base offset into `vertices` (units: vertex slots, not bytes).
    pub vertex_offset: u32,
    /// Base offset into `meshlet_vertices` (units: u32 entries).
    pub meshlet_vertex_offset: u32,
    /// Base byte offset into `meshlet_triangles`.
    pub meshlet_triangle_offset: u32,
    /// Pool-global base id this mesh's group_index values were shifted
    /// by at registration. The shader subtracts it to recover the
    /// mesh-local group id when computing the per-instance slot in
    /// `group_max_err` (#474). `0` for meshes with no LOD groups.
    pub group_base: u32,
    /// Number of distinct group_ids this mesh contributes (`max_local +
    /// 1`). Used by the CPU prefix-sum that lays out each instance's
    /// reserved range in `group_max_err` (#474). `0` for meshes
    /// without LOD groups.
    pub group_count: u32,
    pub _pad0: u32,
}

impl MeshDescriptor {
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

/// Opaque handle returned from [`GlobalMeshPool::register`](super::GlobalMeshPool::register).
/// The `mesh_id` is what `MeshInstance::mesh_id` should hold when the
/// scene cull dispatch fans out over instances.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct MeshHandle {
    pub mesh_id: u32,
}

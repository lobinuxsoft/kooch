//! [`GlobalMeshPool`] — CPU-side accumulator. Call [`Self::register`]
//! once per meshlet mesh, then upload via [`super::GpuGlobalMeshPool`].

use crate::mesh::MeshVertex;

use crate::meshlet::asset::{MeshletDescriptor, MeshletMesh};

use super::descriptor::{MeshDescriptor, MeshHandle};

/// CPU-side accumulator. Call [`Self::register`] once per meshlet mesh,
/// then [`super::GpuGlobalMeshPool::upload`] to push the latest state to
/// GPU. Keep the resulting `GpuGlobalMeshPool` alive across frames.
#[derive(Debug, Default)]
pub struct GlobalMeshPool {
    pub mesh_descriptors: Vec<MeshDescriptor>,
    pub meshlets: Vec<MeshletDescriptor>,
    pub vertices: Vec<MeshVertex>,
    pub meshlet_vertices: Vec<u32>,
    pub meshlet_triangles: Vec<u8>,
    /// Sum of `1 + max_group_id` across registered meshes — the size
    /// of the per-frame `group_max_err` buffer the 2-pass cull (#465)
    /// indexes by `MeshletDescriptor::group_index`. Pooled meshes
    /// each generate group_ids starting at 0; [`Self::register`]
    /// shifts incoming group_ids by the running total so collisions
    /// across meshes cannot occur.
    pub group_capacity: u32,
}

impl GlobalMeshPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of registered meshes.
    pub fn mesh_count(&self) -> u32 {
        self.mesh_descriptors.len() as u32
    }

    /// Maximum meshlets across registered meshes — used as the
    /// worst-case dispatch stride by `cs_cull_scene_pool` so the same
    /// 1D shape covers every mesh + per-thread bounds check.
    pub fn max_meshlets_per_mesh(&self) -> u32 {
        self.mesh_descriptors
            .iter()
            .map(|m| m.meshlet_count)
            .max()
            .unwrap_or(0)
    }

    /// Approximate bytes the persistent storage of this pool occupies
    /// when uploaded to wgpu (vertex + meshlet index + triangle byte
    /// pool + descriptor pool). Used by the perf HUD's VRAM-tracked
    /// counter (#463.5). Not every byte will hit GPU memory exactly
    /// (alignment / driver padding can grow it), but the value is
    /// the closest portable estimate.
    pub fn byte_size(&self) -> u64 {
        let v = (self.vertices.len() * std::mem::size_of::<crate::mesh::MeshVertex>()) as u64;
        let mlv = (self.meshlet_vertices.len() * std::mem::size_of::<u32>()) as u64;
        let mlt = self.meshlet_triangles.len() as u64;
        let m = (self.meshlets.len() * std::mem::size_of::<MeshletDescriptor>()) as u64;
        let md = (self.mesh_descriptors.len() * std::mem::size_of::<MeshDescriptor>()) as u64;
        v + mlv + mlt + m + md
    }

    /// Appends `mesh` to the pool. Returns the handle whose `mesh_id`
    /// the caller should write into `MeshInstance::mesh_id`.
    pub fn register(&mut self, mesh: &MeshletMesh) -> MeshHandle {
        let mesh_id = self.mesh_descriptors.len() as u32;

        let vertex_offset = self.vertices.len() as u32;
        let meshlet_vertex_offset = self.meshlet_vertices.len() as u32;
        let meshlet_triangle_offset = self.meshlet_triangles.len() as u32;
        let first_meshlet = self.meshlets.len() as u32;
        let meshlet_count = mesh.meshlet_count();

        self.vertices.extend_from_slice(&mesh.vertices);
        // meshlet_vertices stores indices INTO the global vertex pool.
        // Each appended value must be shifted by `vertex_offset`
        // (where this mesh's vertices land in the pool) so the GPU
        // shader's `vertices[meshlet_vertices[...]]` lookup hits the
        // correct mesh's vertex slice. Without this rebase the second
        // and later registered meshes silently read vertices from the
        // first mesh — geometry collapses into apparent random spikes.
        self.meshlet_vertices.extend(
            mesh.meshlet_vertices
                .iter()
                .map(|&local_index| local_index + vertex_offset),
        );
        self.meshlet_triangles
            .extend_from_slice(&mesh.meshlet_triangles);

        // Pad triangles up to a 4-byte boundary so the next mesh's
        // first triangle byte lands on a u32 word boundary — the cull
        // shader reads `array<u32>` and extracts bytes via shift-mask;
        // an unaligned base offset would mis-read the first byte.
        while self.meshlet_triangles.len() % 4 != 0 {
            self.meshlet_triangles.push(0);
        }

        // Re-base each meshlet's offsets into pool-global coordinates.
        // Three indices need shifting on top of the byte-offset
        // rebases above:
        //
        // - `parent_meshlet_index`: a meshlet index INTO the same
        //   chain. Without `+ first_meshlet`, mesh #2's parent ids
        //   silently reference mesh #1's slice in the concatenated
        //   pool — the 2-pass cull then reads wrong parents in pass 1
        //   and the LOD selector behaves randomly per mesh.
        // - `group_index` / `children_group_index`: every mesh's
        //   builder allocated group ids starting at 0, so without
        //   shifting they would collide across meshes in the shared
        //   `group_max_err` buffer the 2-pass cull (#465) keys by
        //   these ids.
        let group_offset = self.group_capacity;
        let mut max_local_group: i64 = -1;
        for desc in &mesh.meshlets {
            let new_parent =
                if desc.parent_meshlet_index == crate::meshlet::asset::MESHLET_ROOT_PARENT {
                    desc.parent_meshlet_index
                } else {
                    desc.parent_meshlet_index + first_meshlet
                };
            let new_group_index = if desc.group_index == crate::meshlet::asset::MESHLET_GROUP_NONE {
                desc.group_index
            } else {
                max_local_group = max_local_group.max(desc.group_index as i64);
                desc.group_index + group_offset
            };
            let new_children_group_index =
                if desc.children_group_index == crate::meshlet::asset::MESHLET_GROUP_NONE {
                    desc.children_group_index
                } else {
                    max_local_group = max_local_group.max(desc.children_group_index as i64);
                    desc.children_group_index + group_offset
                };
            self.meshlets.push(MeshletDescriptor {
                vertex_offset: desc.vertex_offset + meshlet_vertex_offset,
                triangle_offset: desc.triangle_offset + meshlet_triangle_offset,
                parent_meshlet_index: new_parent,
                group_index: new_group_index,
                children_group_index: new_children_group_index,
                ..*desc
            });
        }
        let group_count = if max_local_group >= 0 {
            let count = (max_local_group as u32) + 1;
            self.group_capacity += count;
            count
        } else {
            0
        };

        self.mesh_descriptors.push(MeshDescriptor {
            first_meshlet,
            meshlet_count,
            vertex_offset,
            meshlet_vertex_offset,
            meshlet_triangle_offset,
            group_base: group_offset,
            group_count,
            _pad0: 0,
        });

        MeshHandle { mesh_id }
    }
}

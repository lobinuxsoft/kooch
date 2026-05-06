//! Global mesh pool — concatenated meshlet/vertex/triangle storage
//! shared by every registered `MeshletMesh`. Phase 1.E.1b: lets the
//! scene-wide cull dispatch enumerate meshlets across **different**
//! meshes via a `mesh_id` indirection, instead of being locked to a
//! single registered mesh as in 1.E.1.
//!
//! # Layout
//!
//! Each registered mesh appends to four flat CPU arrays:
//! - `meshlets`: `Vec<MeshletDescriptor>` — per-mesh meshlet metadata
//!   (offsets re-based into pool-global coordinates).
//! - `vertices`: `Vec<MeshVertex>` — concatenated vertex pools.
//! - `meshlet_vertices`: `Vec<u32>` — concatenated meshlet→pool
//!   indices, rebased to point into this pool's `vertices`.
//! - `meshlet_triangles`: `Vec<u8>` — concatenated raw triangle bytes
//!   (3 bytes per triangle, padded to 4-byte boundaries between
//!   meshes so future GPU `array<u32>` reads stay aligned).
//!
//! A parallel `mesh_descriptors: Vec<MeshDescriptor>` carries each
//! mesh's `(first_meshlet, meshlet_count, vertex_offset,
//! meshlet_vertex_offset, meshlet_triangle_offset)`. The GPU reads
//! this descriptor at `inst.mesh_id` to redirect every per-meshlet
//! lookup.

mod gpu;

pub use gpu::GpuGlobalMeshPool;

use bytemuck::{Pod, Zeroable};

use crate::mesh::MeshVertex;

use super::asset::{MeshletDescriptor, MeshletMesh};

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

/// Opaque handle returned from [`GlobalMeshPool::register`]. The
/// `mesh_id` is what `MeshInstance::mesh_id` should hold when the
/// scene cull dispatch fans out over instances.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct MeshHandle {
    pub mesh_id: u32,
}

/// CPU-side accumulator. Call [`Self::register`] once per meshlet mesh,
/// then [`Self::upload`] to push the latest state to GPU. Keep the
/// resulting `GpuGlobalMeshPool` alive across frames.
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
            let new_parent = if desc.parent_meshlet_index
                == crate::meshlet::asset::MESHLET_ROOT_PARENT
            {
                desc.parent_meshlet_index
            } else {
                desc.parent_meshlet_index + first_meshlet
            };
            let new_group_index = if desc.group_index == crate::meshlet::asset::MESHLET_GROUP_NONE
            {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Mesh;
    use crate::meshlet::build_default_meshlets;

    fn cube_mesh() -> Mesh {
        let positions = [
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
        ];
        let face_normals = [
            [0.0, 0.0, -1.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0],
            [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [1.0, 0.0, 0.0],
        ];
        let face_indices: [[usize; 4]; 6] = [
            [0, 1, 2, 3], [4, 5, 6, 7], [0, 1, 5, 4],
            [3, 2, 6, 7], [0, 3, 7, 4], [1, 2, 6, 5],
        ];
        let mut vertices = Vec::with_capacity(24);
        let mut indices = Vec::with_capacity(36);
        for (face_idx, corners) in face_indices.iter().enumerate() {
            let normal = face_normals[face_idx];
            let base = vertices.len() as u32;
            for &c in corners {
                vertices.push(MeshVertex {
                    position: positions[c],
                    normal,
                    uv: [0.0, 0.0],
                });
            }
            indices.extend_from_slice(&[
                base, base + 1, base + 2,
                base, base + 2, base + 3,
            ]);
        }
        Mesh::from_arrays(vertices, indices)
    }

    #[test]
    fn descriptor_layout_is_pod_32_bytes() {
        assert_eq!(MeshDescriptor::SIZE, 32);
    }

    #[test]
    fn empty_pool_has_zero_meshes() {
        let pool = GlobalMeshPool::new();
        assert_eq!(pool.mesh_count(), 0);
        assert_eq!(pool.max_meshlets_per_mesh(), 0);
    }

    #[test]
    fn register_increments_mesh_id() {
        let mesh = build_default_meshlets(&cube_mesh()).expect("build");
        let mut pool = GlobalMeshPool::new();
        let h0 = pool.register(&mesh);
        let h1 = pool.register(&mesh);
        assert_eq!(h0.mesh_id, 0);
        assert_eq!(h1.mesh_id, 1);
        assert_eq!(pool.mesh_count(), 2);
    }

    #[test]
    fn register_concatenates_arrays_with_correct_offsets() {
        let mesh = build_default_meshlets(&cube_mesh()).expect("build");
        let mut pool = GlobalMeshPool::new();
        let h0 = pool.register(&mesh);
        let descriptor_0 = pool.mesh_descriptors[h0.mesh_id as usize];
        assert_eq!(descriptor_0.vertex_offset, 0);
        assert_eq!(descriptor_0.first_meshlet, 0);

        let len_v_0 = pool.vertices.len() as u32;
        let len_mv_0 = pool.meshlet_vertices.len() as u32;
        let len_meshlets_0 = pool.meshlets.len() as u32;

        let h1 = pool.register(&mesh);
        let descriptor_1 = pool.mesh_descriptors[h1.mesh_id as usize];
        assert_eq!(descriptor_1.vertex_offset, len_v_0);
        assert_eq!(descriptor_1.meshlet_vertex_offset, len_mv_0);
        assert_eq!(descriptor_1.first_meshlet, len_meshlets_0);
    }

    #[test]
    fn meshlet_offsets_are_rebased_into_pool_coordinates() {
        let mesh = build_default_meshlets(&cube_mesh()).expect("build");
        let mut pool = GlobalMeshPool::new();
        pool.register(&mesh);
        let len_mv = pool.meshlet_vertices.len() as u32;
        let len_mt = pool.meshlet_triangles.len() as u32;
        pool.register(&mesh);
        let second_slice_start = pool.meshlets.len() / 2;
        let mp = pool.meshlets[second_slice_start];
        let original = mesh.meshlets[0];
        assert_eq!(mp.vertex_offset, original.vertex_offset + len_mv);
        assert_eq!(mp.triangle_offset, original.triangle_offset + len_mt);
    }

    #[test]
    fn parent_meshlet_index_values_are_rebased_into_pool_meshlet_space() {
        // Regression: `parent_meshlet_index` is a meshlet index into
        // the SAME chain. When the pool concatenates a second mesh,
        // its parents land inside the second mesh's slice — the
        // values must be shifted by the first_meshlet offset so the
        // 2-pass cull's pass 1 reads the correct parent's pixel
        // error. Before the fix, mesh #2's parents silently
        // referenced mesh #1's meshlets and the LOD selector
        // behaved randomly per mesh.
        use crate::meshlet::asset::MESHLET_ROOT_PARENT;
        let mesh = build_default_meshlets(&cube_mesh()).expect("build");
        let mut pool = GlobalMeshPool::new();
        pool.register(&mesh);
        let first_mesh_meshlet_count = pool.meshlets.len() as u32;
        pool.register(&mesh);

        // Every parent index in the second registration's slice must
        // be either the root sentinel or land in the second mesh's
        // own range [first_mesh_meshlet_count, total).
        for m in &pool.meshlets[first_mesh_meshlet_count as usize..] {
            if m.parent_meshlet_index == MESHLET_ROOT_PARENT {
                continue;
            }
            assert!(
                m.parent_meshlet_index >= first_mesh_meshlet_count,
                "parent index {} in second mesh's slice must be >= {}",
                m.parent_meshlet_index,
                first_mesh_meshlet_count,
            );
            assert!(
                (m.parent_meshlet_index as usize) < pool.meshlets.len(),
                "parent index {} must stay within the pool",
                m.parent_meshlet_index,
            );
        }
    }

    #[test]
    fn meshlet_vertices_values_are_rebased_into_pool_vertex_space() {
        // Regression: the pool used to extend_from_slice(meshlet_vertices)
        // verbatim, which left the second mesh's local indices pointing
        // back at the first mesh's vertices in the concatenated pool.
        // The shader's vertices[meshlet_vertices[..]] lookup then
        // produced random geometry. The fix shifts each value by the
        // mesh's vertex base offset on append.
        let mesh = build_default_meshlets(&cube_mesh()).expect("build");
        let mut pool = GlobalMeshPool::new();
        pool.register(&mesh);
        let first_mesh_vertex_count = pool.vertices.len() as u32;
        let first_mesh_meshlet_vertex_count = pool.meshlet_vertices.len();
        pool.register(&mesh);

        // Every value the second registration appended must reach into
        // the pool slice [first_mesh_vertex_count, 2 * first_mesh_vertex_count).
        for &v in &pool.meshlet_vertices[first_mesh_meshlet_vertex_count..] {
            assert!(
                v >= first_mesh_vertex_count,
                "meshlet_vertices value {v} from second mesh must point past the first mesh's slice (>= {first_mesh_vertex_count})",
            );
            assert!(
                (v as usize) < pool.vertices.len(),
                "meshlet_vertices value {v} must stay within the pool's vertex range ({})",
                pool.vertices.len(),
            );
        }
    }
}

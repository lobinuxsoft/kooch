//! [`GlobalMeshPool`] — CPU-side accumulator. Call [`Self::register`]
//! once per meshlet mesh, then upload via [`super::GpuGlobalMeshPool`].

use crate::mesh::MeshVertex;

use crate::meshlet::asset::{MeshletDescriptor, MeshletMesh};

use super::descriptor::{MeshDescriptor, MeshHandle};
use glam::{Mat4, Vec3};

/// A mesh's extent, as one sphere in mesh space.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct MeshBounds {
    pub center: Vec3,
    pub radius: f32,
}

impl MeshBounds {
    /// The union of every meshlet's AABB, as a sphere.
    pub fn of_meshlets(meshlets: &[MeshletDescriptor]) -> Self {
        let Some(first) = meshlets.first() else {
            return Self::default();
        };
        let mut min = Vec3::from(first.aabb_min);
        let mut max = Vec3::from(first.aabb_max);
        for meshlet in &meshlets[1..] {
            min = min.min(Vec3::from(meshlet.aabb_min));
            max = max.max(Vec3::from(meshlet.aabb_max));
        }
        Self {
            center: (min + max) * 0.5,
            radius: (max - min).length() * 0.5,
        }
    }

    /// The same sphere after `transform`, in world space.
    pub fn transformed(&self, transform: Mat4) -> (Vec3, f32) {
        let center = transform.transform_point3(self.center);
        let scale = transform
            .x_axis
            .truncate()
            .length()
            .max(transform.y_axis.truncate().length())
            .max(transform.z_axis.truncate().length());
        (center, self.radius * scale)
    }
}

/// CPU-side accumulator. Call [`Self::register`] once per meshlet mesh,
/// then `super::GpuGlobalMeshPool::upload` to push the latest state to
/// GPU. Keep the resulting `GpuGlobalMeshPool` alive across frames.
#[derive(Debug, Default)]
pub struct GlobalMeshPool {
    pub mesh_descriptors: Vec<MeshDescriptor>,
    pub meshlets: Vec<MeshletDescriptor>,
    pub vertices: Vec<MeshVertex>,
    pub meshlet_vertices: Vec<u32>,
    pub meshlet_triangles: Vec<u8>,
    /// One bounding sphere per registered mesh, in MESH space, parallel to `mesh_descriptors`
    /// (#847).
    pub mesh_bounds: Vec<MeshBounds>,
    /// Sum of `1 + max_group_id` across registered meshes — the size of the per-frame
    /// `group_max_err` buffer the 2-pass cull (#465) indexes by `MeshletDescriptor::group_index`.
    pub group_capacity: u32,
}

/// One mesh's slice of each parallel array.
struct MeshSlices<'a> {
    vertices: &'a [MeshVertex],
    meshlet_vertices: &'a [u32],
    meshlet_triangles: &'a [u8],
    meshlets: &'a [MeshletDescriptor],
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

    /// Approximate bytes the persistent storage of this pool occupies when uploaded to wgpu (vertex
    /// + meshlet index + triangle byte pool + descriptor pool). Used by the perf HUD's VRAM-tracked
    /// counter (#463.5).
    pub fn byte_size(&self) -> u64 {
        let v = (self.vertices.len() * std::mem::size_of::<crate::mesh::MeshVertex>()) as u64;
        let mlv = (self.meshlet_vertices.len() * std::mem::size_of::<u32>()) as u64;
        let mlt = self.meshlet_triangles.len() as u64;
        let m = (self.meshlets.len() * std::mem::size_of::<MeshletDescriptor>()) as u64;
        let md = (self.mesh_descriptors.len() * std::mem::size_of::<MeshDescriptor>()) as u64;
        v + mlv + mlt + m + md
    }

    /// Bytes this one mesh occupies, counted the same way as [`Self::byte_size`].
    ///
    /// Its slices run from its own offsets to the next mesh's, because `register` only ever
    /// appends — the last mesh runs to the end of each array.
    pub fn mesh_bytes(&self, mesh_id: u32) -> u64 {
        let Some(range) = self.ranges(mesh_id) else {
            return 0;
        };
        (range.vertices.len() * std::mem::size_of::<crate::mesh::MeshVertex>()
            + range.meshlet_vertices.len() * std::mem::size_of::<u32>()
            + range.meshlet_triangles.len()
            + range.meshlets.len() * std::mem::size_of::<MeshletDescriptor>()
            + std::mem::size_of::<MeshDescriptor>()) as u64
    }

    /// Repacks the pool down to `live`, dropping everything else, and returns each kept mesh's
    /// new id in the order given.
    ///
    /// 🔴 Ids move. Every `MeshInstance::mesh_id` is resolved from the registry each frame, so the
    /// caller only has to repoint the registry in the same breath — nothing else holds an id
    /// across frames.
    pub fn retain(&mut self, live: &[u32]) -> Vec<u32> {
        let kept: Vec<MeshletMesh> = live.iter().filter_map(|id| self.extract(*id)).collect();
        *self = Self::new();
        kept.iter()
            .map(|mesh| self.register(mesh).mesh_id)
            .collect()
    }

    /// One mesh lifted back out of the pool, with every offset returned to mesh-local.
    ///
    /// Goes back through [`Self::register`] on the way in, so the rebasing arithmetic lives in
    /// one place and cannot drift between the two directions.
    fn extract(&self, mesh_id: u32) -> Option<MeshletMesh> {
        let range = self.ranges(mesh_id)?;
        let descriptor = &self.mesh_descriptors[mesh_id as usize];
        let mut aabb = crate::mesh::Aabb::empty();
        let meshlets = range
            .meshlets
            .iter()
            .map(|desc| {
                aabb.expand(glam::Vec3::from(desc.aabb_min));
                aabb.expand(glam::Vec3::from(desc.aabb_max));
                MeshletDescriptor {
                    vertex_offset: desc.vertex_offset - descriptor.meshlet_vertex_offset,
                    triangle_offset: desc.triangle_offset - descriptor.meshlet_triangle_offset,
                    parent_meshlet_index: match desc.parent_meshlet_index {
                        crate::meshlet::asset::MESHLET_ROOT_PARENT => desc.parent_meshlet_index,
                        parent => parent - descriptor.first_meshlet,
                    },
                    group_index: match desc.group_index {
                        crate::meshlet::asset::MESHLET_GROUP_NONE => desc.group_index,
                        group => group - descriptor.group_base,
                    },
                    children_group_index: match desc.children_group_index {
                        crate::meshlet::asset::MESHLET_GROUP_NONE => desc.children_group_index,
                        group => group - descriptor.group_base,
                    },
                    ..*desc
                }
            })
            .collect();
        Some(MeshletMesh {
            vertices: range.vertices.to_vec(),
            meshlet_vertices: range
                .meshlet_vertices
                .iter()
                .map(|index| index - descriptor.vertex_offset)
                .collect(),
            meshlet_triangles: range.meshlet_triangles.to_vec(),
            meshlets,
            aabb,
        })
    }

    /// Where one mesh's data sits in each of the parallel arrays.
    fn ranges(&self, mesh_id: u32) -> Option<MeshSlices<'_>> {
        let descriptor = self.mesh_descriptors.get(mesh_id as usize)?;
        let next = self.mesh_descriptors.get(mesh_id as usize + 1);
        let end = |of: fn(&MeshDescriptor) -> u32, total: usize| {
            next.map_or(total, |descriptor| of(descriptor) as usize)
        };
        Some(MeshSlices {
            vertices: &self.vertices
                [descriptor.vertex_offset as usize..end(|d| d.vertex_offset, self.vertices.len())],
            meshlet_vertices: &self.meshlet_vertices[descriptor.meshlet_vertex_offset as usize
                ..end(|d| d.meshlet_vertex_offset, self.meshlet_vertices.len())],
            meshlet_triangles: &self.meshlet_triangles[descriptor.meshlet_triangle_offset as usize
                ..end(|d| d.meshlet_triangle_offset, self.meshlet_triangles.len())],
            meshlets: &self.meshlets[descriptor.first_meshlet as usize
                ..(descriptor.first_meshlet + descriptor.meshlet_count) as usize],
        })
    }

    /// Appends `mesh` to the pool. Returns the handle whose `mesh_id`
    /// the caller should write into `MeshInstance::mesh_id`.
    pub fn register(&mut self, mesh: &MeshletMesh) -> MeshHandle {
        let mesh_id = self.mesh_descriptors.len() as u32;
        self.mesh_bounds
            .push(MeshBounds::of_meshlets(&mesh.meshlets));

        let vertex_offset = self.vertices.len() as u32;
        let meshlet_vertex_offset = self.meshlet_vertices.len() as u32;
        let meshlet_triangle_offset = self.meshlet_triangles.len() as u32;
        let first_meshlet = self.meshlets.len() as u32;
        let meshlet_count = mesh.meshlet_count();

        self.vertices.extend_from_slice(&mesh.vertices);
        // meshlet_vertices stores indices INTO the global vertex pool. Each appended value must be
        // shifted by `vertex_offset` (where this mesh's vertices land in the pool) so the GPU
        // shader's `vertices[meshlet_vertices[...]]` lookup hits the correct mesh's vertex slice.
        self.meshlet_vertices.extend(
            mesh.meshlet_vertices
                .iter()
                .map(|&local_index| local_index + vertex_offset),
        );
        self.meshlet_triangles
            .extend_from_slice(&mesh.meshlet_triangles);

        // Pad triangles up to a 4-byte boundary so the next mesh's first triangle byte lands on a
        // u32 word boundary — the cull shader reads `array<u32>` and extracts bytes via shift-mask;
        // an unaligned base offset would mis-read the first byte.
        while self.meshlet_triangles.len() % 4 != 0 {
            self.meshlet_triangles.push(0);
        }

        // Re-base each meshlet's offsets into pool-global coordinates. Three indices need shifting
        // on top of the byte-offset rebases above.
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

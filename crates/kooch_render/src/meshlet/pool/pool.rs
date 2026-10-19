//! [`GlobalMeshPool`] — CPU-side accumulator. Call [`Self::register`]
//! once per meshlet mesh, then upload via [`super::GpuGlobalMeshPool`].

use crate::mesh::MeshVertex;

use crate::meshlet::asset::{
    MESHLET_GROUP_NONE, MESHLET_ROOT_PARENT, MeshletDescriptor, MeshletMesh,
};

use super::descriptor::{MeshDescriptor, MeshHandle};
use super::slots::{Holes, Slot, Span};
use bytemuck::Zeroable;
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
#[derive(Debug)]
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
    /// Where each mesh sits, parallel to `mesh_descriptors`. A dead entry is a hole.
    slots: Vec<Slot>,
    free_vertices: Holes,
    free_meshlet_vertices: Holes,
    free_triangles: Holes,
    free_meshlets: Holes,
    free_groups: Holes,
    /// `mesh_descriptors` indices a released mesh gave back.
    free_ids: Vec<u32>,
}

impl Default for GlobalMeshPool {
    fn default() -> Self {
        Self {
            mesh_descriptors: Vec::new(),
            meshlets: Vec::new(),
            vertices: Vec::new(),
            meshlet_vertices: Vec::new(),
            meshlet_triangles: Vec::new(),
            mesh_bounds: Vec::new(),
            group_capacity: 0,
            slots: Vec::new(),
            free_vertices: Holes::aligned(1),
            free_meshlet_vertices: Holes::aligned(1),
            // The cull shader reads triangle bytes out of an `array<u32>`, so every mesh's base
            // offset has to land on a word boundary.
            free_triangles: Holes::aligned(4),
            free_meshlets: Holes::aligned(1),
            free_groups: Holes::aligned(1),
            free_ids: Vec::new(),
        }
    }
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

    /// Where one mesh's data sits in each of the parallel arrays, or `None` for a hole.
    fn ranges(&self, mesh_id: u32) -> Option<MeshSlices<'_>> {
        let slot = self.slots.get(mesh_id as usize).filter(|slot| slot.live)?;
        Some(MeshSlices {
            vertices: &self.vertices[slot.vertices.range()],
            meshlet_vertices: &self.meshlet_vertices[slot.meshlet_vertices.range()],
            meshlet_triangles: &self.meshlet_triangles[slot.triangles.range()],
            meshlets: &self.meshlets[slot.meshlets.range()],
        })
    }

    /// Bytes the free lists are holding: ranges a released mesh left, waiting for one that fits.
    pub fn free_bytes(&self) -> u64 {
        self.free_vertices.free_slots() * std::mem::size_of::<MeshVertex>() as u64
            + self.free_meshlet_vertices.free_slots() * std::mem::size_of::<u32>() as u64
            + self.free_triangles.free_slots()
            + self.free_meshlets.free_slots() * std::mem::size_of::<MeshletDescriptor>() as u64
            + self.free_ids.len() as u64 * std::mem::size_of::<MeshDescriptor>() as u64
    }

    /// Hands one mesh's ranges and its id back, for the next [`Self::register`] to reuse.
    ///
    /// 🔴 The caller has to have stopped resolving this `mesh_id` first. The id is handed out
    /// again on the very next register, and an instance still holding it would draw the new mesh.
    pub fn release(&mut self, mesh_id: u32) {
        let Some(slot) = self
            .slots
            .get_mut(mesh_id as usize)
            .filter(|slot| slot.live)
        else {
            return;
        };
        let slot = std::mem::take(slot);
        self.free_vertices.give(slot.vertices);
        self.free_meshlet_vertices.give(slot.meshlet_vertices);
        self.free_triangles.give(slot.triangles);
        self.free_meshlets.give(slot.meshlets);
        self.free_groups.give(slot.groups);
        // A hole still ships to the GPU, so it has to cull to nothing: zero meshlets.
        self.mesh_descriptors[mesh_id as usize] = MeshDescriptor::zeroed();
        self.mesh_bounds[mesh_id as usize] = MeshBounds::default();
        self.free_ids.push(mesh_id);
    }

    /// Puts `mesh` in the pool, reusing the ranges a released mesh left wherever they fit.
    /// Returns the handle whose `mesh_id` the caller should write into `MeshInstance::mesh_id`.
    pub fn register(&mut self, mesh: &MeshletMesh) -> MeshHandle {
        let slot = self.reserve(mesh);
        let mesh_id = match self.free_ids.pop() {
            Some(id) => id,
            None => {
                self.mesh_descriptors.push(MeshDescriptor::zeroed());
                self.mesh_bounds.push(MeshBounds::default());
                self.slots.push(Slot::default());
                self.mesh_descriptors.len() as u32 - 1
            }
        };
        self.write(mesh_id, mesh, slot);
        MeshHandle { mesh_id }
    }

    /// One range per array for `mesh`: out of the holes when it fits, off the end when it does not.
    fn reserve(&mut self, mesh: &MeshletMesh) -> Slot {
        let vertices = take(
            &mut self.free_vertices,
            &mut self.vertices,
            mesh.vertices.len(),
            MeshVertex::zeroed(),
        );
        let meshlet_vertices = take(
            &mut self.free_meshlet_vertices,
            &mut self.meshlet_vertices,
            mesh.meshlet_vertices.len(),
            0,
        );
        let triangles = take(
            &mut self.free_triangles,
            &mut self.meshlet_triangles,
            mesh.meshlet_triangles.len(),
            0,
        );
        let meshlets = take(
            &mut self.free_meshlets,
            &mut self.meshlets,
            mesh.meshlets.len(),
            MeshletDescriptor::zeroed(),
        );
        // Groups are a capacity the cull indexes into, not an array this pool stores.
        let count = group_count(mesh);
        let groups = match self.free_groups.take(count) {
            Some(span) => span,
            None => {
                let span = Span {
                    start: self.group_capacity,
                    len: count,
                };
                self.group_capacity += count;
                span
            }
        };
        Slot {
            vertices,
            meshlet_vertices,
            triangles,
            meshlets,
            groups,
            live: true,
        }
    }

    /// Copies `mesh` into its reserved ranges, rebasing every index into pool-global coordinates.
    ///
    /// `meshlet_vertices` holds indices INTO the global vertex array, meshlet offsets are
    /// pool-relative, and group ids are shifted by the mesh's `group_base` — rebase any one of
    /// them wrong and the shader reads another mesh's geometry.
    fn write(&mut self, mesh_id: u32, mesh: &MeshletMesh, slot: Slot) {
        self.vertices[slot.vertices.range()].copy_from_slice(&mesh.vertices);
        for (at, local) in slot.meshlet_vertices.range().zip(&mesh.meshlet_vertices) {
            self.meshlet_vertices[at] = local + slot.vertices.start;
        }
        // The span is rounded up to a word; the tail past the mesh's own bytes is padding nothing
        // reads, because each meshlet's triangle_offset bounds it.
        let bytes = slot.triangles.start as usize
            ..slot.triangles.start as usize + mesh.meshlet_triangles.len();
        self.meshlet_triangles[bytes].copy_from_slice(&mesh.meshlet_triangles);
        for (at, desc) in slot.meshlets.range().zip(&mesh.meshlets) {
            self.meshlets[at] = MeshletDescriptor {
                vertex_offset: desc.vertex_offset + slot.meshlet_vertices.start,
                triangle_offset: desc.triangle_offset + slot.triangles.start,
                parent_meshlet_index: match desc.parent_meshlet_index {
                    MESHLET_ROOT_PARENT => desc.parent_meshlet_index,
                    parent => parent + slot.meshlets.start,
                },
                group_index: shifted(desc.group_index, slot.groups.start),
                children_group_index: shifted(desc.children_group_index, slot.groups.start),
                ..*desc
            };
        }
        self.mesh_bounds[mesh_id as usize] = MeshBounds::of_meshlets(&mesh.meshlets);
        self.mesh_descriptors[mesh_id as usize] = MeshDescriptor {
            first_meshlet: slot.meshlets.start,
            meshlet_count: slot.meshlets.len,
            vertex_offset: slot.vertices.start,
            meshlet_vertex_offset: slot.meshlet_vertices.start,
            meshlet_triangle_offset: slot.triangles.start,
            group_base: slot.groups.start,
            group_count: slot.groups.len,
            _pad0: 0,
        };
        self.slots[mesh_id as usize] = slot;
    }
}

/// A span out of `holes`, or one appended to `array` — which grows with filler the write
/// overwrites.
fn take<T: Copy>(holes: &mut Holes, array: &mut Vec<T>, len: usize, filler: T) -> Span {
    let len = holes.rounded(len as u32);
    if let Some(span) = holes.take(len) {
        return span;
    }
    let span = Span {
        start: array.len() as u32,
        len,
    };
    array.resize(span.end() as usize, filler);
    span
}

/// Distinct LOD group ids `mesh` contributes, counted over both the group a meshlet belongs to and
/// the one its children form.
fn group_count(mesh: &MeshletMesh) -> u32 {
    mesh.meshlets
        .iter()
        .flat_map(|desc| [desc.group_index, desc.children_group_index])
        .filter(|group| *group != MESHLET_GROUP_NONE)
        .max()
        .map_or(0, |group| group + 1)
}

fn shifted(group: u32, base: u32) -> u32 {
    match group {
        MESHLET_GROUP_NONE => group,
        group => group + base,
    }
}

//! Meshes the GPU writes itself (#313): the CPU reserves their ranges and their descriptor, and
//! never uploads or moves the payload, because only the GPU holds it.

use bytemuck::Zeroable;

use crate::mesh::MeshVertex;
use crate::meshlet::asset::MeshletDescriptor;

use super::descriptor::{MeshDescriptor, MeshHandle};
use super::pool::{GlobalMeshPool, MeshBounds, take};
use super::slots::{Slot, Span};

/// A GPU-written mesh a repack carried to new ranges, waiting for the mirror to copy it.
#[derive(Copy, Clone, Debug)]
pub(super) struct Move {
    pub from: Slot,
    pub to: Slot,
}

/// How much of each array a GPU-written mesh takes. `triangle_bytes` is rounded up to a word.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct GpuMeshSizes {
    pub vertices: u32,
    pub meshlet_vertices: u32,
    pub triangle_bytes: u32,
    pub meshlets: u32,
}

impl GlobalMeshPool {
    /// Reserves a mesh the GPU will fill. Its descriptor names where each array's range starts;
    /// the writer's meshlets index the pool, not the mesh, as [`Self::register`] rebases them.
    pub fn reserve_gpu(&mut self, sizes: GpuMeshSizes, bounds: MeshBounds) -> MeshHandle {
        let slot = Slot {
            vertices: take(
                &mut self.free_vertices,
                &mut self.vertices,
                sizes.vertices as usize,
                MeshVertex::zeroed(),
            ),
            meshlet_vertices: take(
                &mut self.free_meshlet_vertices,
                &mut self.meshlet_vertices,
                sizes.meshlet_vertices as usize,
                0,
            ),
            triangles: take(
                &mut self.free_triangles,
                &mut self.meshlet_triangles,
                sizes.triangle_bytes as usize,
                0,
            ),
            meshlets: take(
                &mut self.free_meshlets,
                &mut self.meshlets,
                sizes.meshlets as usize,
                MeshletDescriptor::zeroed(),
            ),
            groups: Span::default(),
            live: true,
            gpu: true,
        };
        let mesh_id = self.next_id();
        self.place(mesh_id, slot, bounds);
        MeshHandle { mesh_id }
    }

    /// Whether the GPU writes this mesh's payload.
    pub fn gpu_owned(&self, mesh_id: u32) -> bool {
        self.slots
            .get(mesh_id as usize)
            .is_some_and(|slot| slot.live && slot.gpu)
    }

    /// Reserves fresh ranges for `old`'s GPU-written mesh `id`, and records the move the mirror
    /// owes it: only the GPU holds the payload, so the GPU carries it (#773).
    pub(super) fn carry(&mut self, old: &GlobalMeshPool, id: u32) -> Option<u32> {
        let at = *old.slots.get(id as usize)?;
        let sizes = GpuMeshSizes {
            vertices: at.vertices.len,
            meshlet_vertices: at.meshlet_vertices.len,
            triangle_bytes: at.triangles.len,
            meshlets: at.meshlets.len,
        };
        let bounds = old
            .mesh_bounds
            .get(id as usize)
            .copied()
            .unwrap_or_default();
        let mesh_id = self.reserve_gpu(sizes, bounds).mesh_id;
        let to = *self.slots.get(mesh_id as usize)?;
        // A move not run yet still has the payload where it started.
        let from = old
            .moves
            .iter()
            .find(|m| m.to.meshlets == at.meshlets && m.to.vertices == at.vertices)
            .map_or(at, |m| m.from);
        self.moves.push(Move { from, to });
        Some(mesh_id)
    }

    /// Writes the descriptor of a GPU-written mesh; its payload is left to the GPU.
    fn place(&mut self, mesh_id: u32, slot: Slot, bounds: MeshBounds) {
        let at = mesh_id as usize;
        self.mesh_descriptors[at] = MeshDescriptor {
            first_meshlet: slot.meshlets.start,
            meshlet_count: slot.meshlets.len,
            vertex_offset: slot.vertices.start,
            meshlet_vertex_offset: slot.meshlet_vertices.start,
            meshlet_triangle_offset: slot.triangles.start,
            group_base: 0,
            group_count: 0,
            _pad0: 0,
        };
        self.mesh_bounds[at] = bounds;
        self.slots[at] = slot;
        self.dirty.descriptors.mark(Span {
            start: mesh_id,
            len: 1,
        });
    }
}

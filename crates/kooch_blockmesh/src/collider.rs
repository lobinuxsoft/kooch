//! Turning the authoring mesh into what the solver collides against.

use kooch_physics::ColliderMesh;

use crate::BlockMesh;

impl BlockMesh {
    /// The collider for this block: shared positions and the triangles that index them.
    /// Welded, unlike [`to_mesh`](Self::to_mesh): split positions duplicate every edge and a
    /// character catches on the seam.
    pub fn to_collider(&self) -> ColliderMesh {
        ColliderMesh {
            vertices: self.positions().to_vec(),
            indices: self.triangles(),
            ..Default::default()
        }
    }
}

/// A bent mesh as geometry the solver can read (#1537).
///
/// 🔴 Only the finest LOD's triangles. The coarser levels describe the same surface again, and a
/// collider built from all of them is a solid full of overlapping shells that a character catches
/// on with nothing reporting a fault.
pub fn bent_collider(mesh: &kooch_render::meshlet::MeshletMesh) -> ColliderMesh {
    ColliderMesh {
        vertices: mesh
            .vertices
            .iter()
            .map(|vertex| glam::Vec3::from(vertex.position))
            .collect(),
        indices: kooch_render::meshlet::base_triangles(mesh),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests;

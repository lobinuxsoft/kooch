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

#[cfg(test)]
mod tests;

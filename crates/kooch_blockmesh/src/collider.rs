//! Turning the authoring mesh into what the solver collides against.

use kooch_physics::ColliderMesh;

use crate::BlockMesh;

impl BlockMesh {
    /// The collider for this block: the shared positions, and the
    /// triangles that index them.
    ///
    /// Welded, unlike [`to_mesh`](Self::to_mesh). A trimesh built from
    /// split positions has every edge duplicated six ways, and a
    /// character walking across a seam catches on the copy the solver
    /// happens to test second.
    ///
    /// No hull and no parts: a block is authored convex face by face,
    /// and asking for a decomposition of something already simple is
    /// seconds spent to arrive back where we started.
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

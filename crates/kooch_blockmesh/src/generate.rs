//! Generating the render mesh from the authoring mesh.

use glam::{Vec2, Vec3};
use kooch_render::mesh::{Mesh, MeshVertex};

use crate::BlockMesh;

impl BlockMesh {
    /// Generates the render mesh with positions split per face, so every face shades flat. Shared
    /// corners would round a box's edges; the collider takes the welded
    /// [`triangles`](Self::triangles).
    pub fn to_mesh(&self) -> Mesh {
        let mut vertices = Vec::with_capacity(self.face_corners_len());
        let mut indices = Vec::new();

        for index in 0..self.face_count() {
            let Some(face) = self.face(index) else {
                continue;
            };
            let normal = self.face_normal(index).unwrap_or(Vec3::Y);
            let first = vertices.len() as u32;

            for corner in face {
                let position = self.positions()[*corner as usize];
                vertices.push(MeshVertex {
                    position: position.to_array(),
                    normal: normal.to_array(),
                    uv: planar_uv(position, normal).to_array(),
                });
            }

            // Fan over the vertices just pushed; faces are convex.
            for corner in 1..face.len() as u32 - 1 {
                indices.extend_from_slice(&[first, first + corner, first + corner + 1]);
            }
        }

        Mesh::from_arrays(vertices, indices)
    }

    /// Total corners across every face — the exact vertex count
    /// [`to_mesh`](Self::to_mesh) will emit.
    fn face_corners_len(&self) -> usize {
        self.faces().map(<[u32]>::len).sum()
    }
}

/// Projects a position onto the plane its normal faces most, one repeat per world unit.
/// World-space, so a wall stretched from 2 m to 8 m shows four repeats, not stretched texels.
fn planar_uv(position: Vec3, normal: Vec3) -> Vec2 {
    let axis = normal.abs();
    if axis.x >= axis.y && axis.x >= axis.z {
        Vec2::new(position.z, position.y)
    } else if axis.y >= axis.z {
        Vec2::new(position.x, position.z)
    } else {
        Vec2::new(position.x, position.y)
    }
}

#[cfg(test)]
mod tests;

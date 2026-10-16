//! Generating the render mesh from the authoring mesh.

use glam::{Vec2, Vec3};
use kooch_render::mesh::{Mesh, MeshVertex};

use crate::BlockMesh;

/// Faces meeting under this angle share a normal; sharper than it, the edge stays a crease.
///
/// 60° rounds a swept tube of six sides or more and leaves a box's 90° edges — and a square
/// profile's four corners — alone, with nothing to author.
const CREASE: f32 = std::f32::consts::FRAC_PI_3;

impl BlockMesh {
    /// Generates the render mesh with positions split per face-corner, so a crease stays sharp
    /// while a curve shades smooth. The collider takes the welded [`triangles`](Self::triangles).
    pub fn to_mesh(&self) -> Mesh {
        let mut vertices = Vec::with_capacity(self.face_corners_len());
        let mut indices = Vec::new();

        let normals = self.face_normals();
        let around = self.corner_faces();

        for index in 0..self.face_count() {
            let Some(face) = self.face(index) else {
                continue;
            };
            let flat = normals[index];
            let first = vertices.len() as u32;

            for corner in face {
                let position = self.positions()[*corner as usize];
                vertices.push(MeshVertex {
                    position: position.to_array(),
                    normal: around.smoothed(*corner, flat, &normals).to_array(),
                    // The face's normal, not the corner's: a projection axis that swims along a
                    // curve stretches the texels it was chosen to keep square.
                    uv: planar_uv(position, flat).to_array(),
                });
            }

            // Fan over the vertices just pushed; faces are convex.
            for corner in 1..face.len() as u32 - 1 {
                indices.extend_from_slice(&[first, first + corner, first + corner + 1]);
            }
        }

        Mesh::from_arrays(vertices, indices)
    }

    /// One normal per face, in face order. A missing face takes `Y` so the index stays aligned.
    fn face_normals(&self) -> Vec<Vec3> {
        (0..self.face_count())
            .map(|index| self.face_normal(index).unwrap_or(Vec3::Y))
            .collect()
    }

    /// CSR over positions: the faces meeting at each.
    fn corner_faces(&self) -> CornerFaces {
        let mut starts = vec![0_u32; self.positions().len() + 1];
        for face in self.faces() {
            for corner in face {
                starts[*corner as usize + 1] += 1;
            }
        }
        for index in 1..starts.len() {
            starts[index] += starts[index - 1];
        }

        let mut filled = starts.clone();
        let mut faces = vec![0_u32; *starts.last().unwrap_or(&0) as usize];
        for (index, face) in self.faces().enumerate() {
            for corner in face {
                let slot = &mut filled[*corner as usize];
                faces[*slot as usize] = index as u32;
                *slot += 1;
            }
        }
        CornerFaces { starts, faces }
    }

    /// Total corners across every face — the exact vertex count
    /// [`to_mesh`](Self::to_mesh) will emit.
    fn face_corners_len(&self) -> usize {
        self.faces().map(<[u32]>::len).sum()
    }
}

/// Which faces meet at each position, as parallel arrays rather than a map per corner.
struct CornerFaces {
    starts: Vec<u32>,
    faces: Vec<u32>,
}

impl CornerFaces {
    /// `flat` averaged with the faces at `corner` that lie within [`CREASE`] of it.
    ///
    /// Unweighted: a block's faces around a swept ring are the same size, and area weighting buys
    /// nothing for the tearing it hides on a degenerate one.
    fn smoothed(&self, corner: u32, flat: Vec3, normals: &[Vec3]) -> Vec3 {
        let limit = CREASE.cos();
        let span = self.starts[corner as usize] as usize..self.starts[corner as usize + 1] as usize;
        let mut total = Vec3::ZERO;
        for face in &self.faces[span] {
            let normal = normals[*face as usize];
            if normal.dot(flat) >= limit {
                total += normal;
            }
        }
        // `flat` is in the sum unless the mesh has no face there at all.
        total.normalize_or(flat)
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

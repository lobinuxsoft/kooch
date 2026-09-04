//! The authoring mesh: shared positions plus faces that index them.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// An editable polygon mesh. Faces are convex and wound counter-clockwise
/// seen from outside.
///
/// Faces are stored CSR-style: every face's corners are concatenated
/// into `face_corners`, and `face_starts` holds where each one begins
/// plus a trailing sentinel, so face `i` owns
/// `face_corners[face_starts[i]..face_starts[i + 1]]`. One allocation
/// for the whole mesh instead of one per face, and walking every corner
/// of every face is a contiguous scan.
///
/// # These field names are serialised
///
/// They travel in the `.blockmesh.ron` file. Renaming one makes saved
/// levels load a default in its place, silently — see the rename hazard
/// in `code-standards`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlockMesh {
    /// Corner positions, shared between the faces that meet there. The
    /// unit of editing: dragging a vertex moves one entry here and every
    /// face using it follows.
    #[serde(default)]
    positions: Vec<Vec3>,
    /// Every face's corners, concatenated. Each entry indexes
    /// `positions`.
    #[serde(default)]
    face_corners: Vec<u32>,
    /// Where each face begins in `face_corners`, with a trailing
    /// sentinel equal to its length. Length is `face_count() + 1`, and
    /// an empty mesh stores it empty rather than `[0]`.
    #[serde(default)]
    face_starts: Vec<u32>,
}

/// The eight corners of a cuboid, indexed so bit 0 is +X, bit 1 is +Y
/// and bit 2 is +Z.
const CUBOID_CORNERS: [[f32; 3]; 8] = [
    [-1.0, -1.0, -1.0],
    [1.0, -1.0, -1.0],
    [1.0, 1.0, -1.0],
    [-1.0, 1.0, -1.0],
    [-1.0, -1.0, 1.0],
    [1.0, -1.0, 1.0],
    [1.0, 1.0, 1.0],
    [-1.0, 1.0, 1.0],
];

/// The six quads of a cuboid, each wound counter-clockwise seen from
/// outside so the generated normal points away from the centre.
const CUBOID_FACES: [[u32; 4]; 6] = [
    [0, 3, 2, 1], // -Z
    [4, 5, 6, 7], // +Z
    [0, 4, 7, 3], // -X
    [1, 2, 6, 5], // +X
    [0, 1, 5, 4], // -Y
    [3, 7, 6, 2], // +Y
];

impl BlockMesh {
    /// An axis-aligned box centred on the origin, extending `half` along
    /// each axis. The shape every blockout starts from.
    ///
    /// A negative or zero extent is honoured rather than rejected: the
    /// box tool drags a corner past its opposite all the time, and a
    /// degenerate box mid-drag is a normal frame, not an error.
    pub fn cuboid(half: Vec3) -> Self {
        let positions = CUBOID_CORNERS
            .iter()
            .map(|corner| Vec3::from_array(*corner) * half)
            .collect();

        let mut face_corners = Vec::with_capacity(CUBOID_FACES.len() * 4);
        let mut face_starts = Vec::with_capacity(CUBOID_FACES.len() + 1);
        for face in &CUBOID_FACES {
            face_starts.push(face_corners.len() as u32);
            face_corners.extend_from_slice(face);
        }
        face_starts.push(face_corners.len() as u32);

        Self {
            positions,
            face_corners,
            face_starts,
        }
    }

    /// Builds a mesh from shared positions and faces given as corner
    /// index lists.
    ///
    /// Returns `None` when a face names a position that does not exist,
    /// or has fewer than three corners: both make every later operation
    /// index out of bounds, and rejecting at the door beats a panic
    /// three calls deep.
    pub fn from_faces(positions: Vec<Vec3>, faces: &[Vec<u32>]) -> Option<Self> {
        let corners = positions.len() as u32;
        let mut face_corners = Vec::new();
        let mut face_starts = Vec::with_capacity(faces.len() + 1);
        for face in faces {
            if face.len() < 3 || face.iter().any(|corner| *corner >= corners) {
                return None;
            }
            face_starts.push(face_corners.len() as u32);
            face_corners.extend_from_slice(face);
        }
        if !faces.is_empty() {
            face_starts.push(face_corners.len() as u32);
        }

        Some(Self {
            positions,
            face_corners,
            face_starts,
        })
    }

    /// The shared corner positions.
    pub fn positions(&self) -> &[Vec3] {
        &self.positions
    }

    /// How many faces the mesh has.
    pub fn face_count(&self) -> usize {
        self.face_starts.len().saturating_sub(1)
    }

    /// The corners of face `index`, or `None` when it does not exist.
    pub fn face(&self, index: usize) -> Option<&[u32]> {
        let start = *self.face_starts.get(index)? as usize;
        let end = *self.face_starts.get(index + 1)? as usize;
        self.face_corners.get(start..end)
    }

    /// Iterates every face's corners in storage order.
    pub fn faces(&self) -> impl Iterator<Item = &[u32]> {
        (0..self.face_count()).filter_map(|index| self.face(index))
    }

    /// The outward normal of face `index`, or `None` when it does not
    /// exist.
    ///
    /// Newell's method rather than one cross product, because a face
    /// dragged out of plane still has a sensible average normal while a
    /// single corner's cross product would swing with whichever corner
    /// happened to be first.
    pub fn face_normal(&self, index: usize) -> Option<Vec3> {
        let face = self.face(index)?;
        let mut normal = Vec3::ZERO;
        for pair in 0..face.len() {
            let current = self.positions[face[pair] as usize];
            let next = self.positions[face[(pair + 1) % face.len()] as usize];
            normal += (current - next).cross(current + next);
        }
        Some(normal.normalize_or(Vec3::Y))
    }

    /// Triangulates every face as a fan, indexing the shared positions.
    ///
    /// Welded on purpose: this feeds the collider, and a physics trimesh
    /// wants corners that coincide to be one corner. Rendering takes the
    /// split version from [`to_mesh`](Self::to_mesh) instead.
    ///
    /// A fan is correct because faces are convex, which every operator
    /// here preserves.
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        let mut triangles = Vec::new();
        for face in self.faces() {
            for corner in 1..face.len() - 1 {
                triangles.push([face[0], face[corner], face[corner + 1]]);
            }
        }
        triangles
    }
}

#[cfg(test)]
mod tests;

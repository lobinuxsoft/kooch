//! The authoring mesh: shared positions plus faces that index them.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// An editable polygon mesh: convex faces wound counter-clockwise from outside, stored CSR-style so
/// face `i` owns `face_corners[face_starts[i]..face_starts[i + 1]]`.
/// 🔴 These field names are serialised in `.block` files **and in the scene** (#1452); renaming one
/// silently loads a default.
///
/// The three lists are hidden from the Inspector: nobody edits five thousand vertices by hand, and
/// a row per item every frame is what would make a blockout unusable. The gizmos are how they are
/// edited.
#[derive(Debug, Clone, Default, Serialize, Deserialize, kooch_ecs::Reflect)]
pub struct BlockMesh {
    /// Corner positions, shared between the faces that meet there. The
    /// unit of editing: dragging a vertex moves one entry here and every
    /// face using it follows.
    #[serde(default)]
    #[reflect(hidden)]
    pub(crate) positions: Vec<Vec3>,
    /// Every face's corners, concatenated. Each entry indexes
    /// `positions`.
    #[serde(default)]
    #[reflect(hidden)]
    pub(crate) face_corners: Vec<u32>,
    /// Where each face begins in `face_corners`, with a trailing
    /// sentinel equal to its length. Length is `face_count() + 1`, and
    /// an empty mesh stores it empty rather than `[0]`.
    #[serde(default)]
    #[reflect(hidden)]
    pub(crate) face_starts: Vec<u32>,
    /// One id per face, in the same order. What a saved reference to a face points at, since the
    /// position is only stable while nothing removes a face (#1550).
    #[serde(default)]
    #[reflect(hidden)]
    pub(crate) face_ids: Vec<u32>,
    /// The next id to hand out. Never reused: an id that comes back means a different face
    /// answering to a reference that outlived its own.
    #[serde(default)]
    #[reflect(hidden)]
    pub(crate) next_face_id: u32,
}

/// Two meshes are equal when they are the same **shape**.
///
/// 🔴 `face_ids` and `next_face_id` are identity, not geometry, and the one caller that asks whether
/// a mesh changed means geometry (`sweep_sync`). Counting them made every scene written before
/// #1565 differ from its own rebuild for ever — the ids are `#[serde(default)]`, so such a file
/// loads with the lists empty — and the sweep re-sent its block on every load, marking a level
/// unsaved that nobody had touched (#1570).
///
/// This also puts the three answers to "did this change?" back in agreement:
/// [`revision_of`](crate::sync_blocks) and the editor's `digest` already hashed geometry alone.
impl PartialEq for BlockMesh {
    fn eq(&self, other: &Self) -> bool {
        self.positions == other.positions
            && self.face_corners == other.face_corners
            && self.face_starts == other.face_starts
    }
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
    /// An axis-aligned box centred on the origin, extending `half` along each axis. A zero or
    /// negative extent is allowed: a corner dragged past its opposite is a normal frame.
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
            face_ids: (0..CUBOID_FACES.len() as u32).collect(),
            next_face_id: CUBOID_FACES.len() as u32,
            face_starts,
        }
    }

    /// Builds a mesh from shared positions and faces given as corner index lists. `None` when a
    /// face names a missing position or has fewer than three corners.
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
            face_ids: (0..faces.len() as u32).collect(),
            next_face_id: faces.len() as u32,
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

    /// The stable id of face `index`, or `None` when the face does not exist.
    ///
    /// A mesh saved before ids existed has none. Its faces answer with their position, which is
    /// the id they would have been given, so an old scene keeps working without a migration pass.
    pub fn face_id(&self, index: usize) -> Option<u32> {
        if index >= self.face_count() {
            return None;
        }
        Some(
            self.face_ids
                .get(index)
                .copied()
                .unwrap_or_else(|| index as u32),
        )
    }

    /// Where the face carrying `id` is now, or `None` when it is gone — a rebuilt shape keeps no
    /// face it was not given.
    pub fn face_of_id(&self, id: u32) -> Option<usize> {
        (0..self.face_count()).find(|index| self.face_id(*index) == Some(id))
    }

    /// An id nothing has held, for a face about to be appended.
    pub(crate) fn take_face_id(&mut self) -> u32 {
        // `max` covers a mesh loaded without ids: its faces answer with their positions, so the
        // counter has to start past them or the next face would collide with one.
        let id = self.next_face_id.max(self.face_count() as u32);
        self.next_face_id = id + 1;
        id
    }

    /// Iterates every face's corners in storage order.
    pub fn faces(&self) -> impl Iterator<Item = &[u32]> {
        (0..self.face_count()).filter_map(|index| self.face(index))
    }

    /// The outward normal of face `index`, or `None` when it does not exist. Newell's method: a
    /// face dragged out of plane keeps a stable normal where one cross product swings.
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

    /// Every corner the given faces use, each once.
    /// 🔴 Once: a shared corner moved per face would move three times and tear the block along its
    /// seams.
    pub fn corners_of(&self, faces: &[u32]) -> Vec<u32> {
        let mut corners: Vec<u32> = Vec::new();
        for face in faces {
            let Some(face) = self.face(*face as usize) else {
                continue;
            };
            for corner in face {
                if !corners.contains(corner) {
                    corners.push(*corner);
                }
            }
        }
        corners
    }

    /// Moves the given corners by `delta`, in the mesh's own space. Faces sharing a moved corner
    /// follow it.
    pub fn move_corners(&mut self, corners: &[u32], delta: Vec3) {
        for corner in corners {
            if let Some(position) = self.positions.get_mut(*corner as usize) {
                *position += delta;
            }
        }
    }

    /// The average of the given faces' corners, in the mesh's own space. Averages corners, not face
    /// centres, so a shared edge is not weighted twice.
    pub fn centre_of(&self, faces: &[u32]) -> Option<Vec3> {
        self.centre(&self.corners_of(faces))
    }

    /// The average of the given corners — the primitive under [`Self::centre_of`], shared by every
    /// selection kind.
    pub fn centre(&self, corners: &[u32]) -> Option<Vec3> {
        if corners.is_empty() {
            return None;
        }
        let total: Vec3 = corners
            .iter()
            .filter_map(|corner| self.positions.get(*corner as usize))
            .sum();
        Some(total / corners.len() as f32)
    }

    /// Turns the given corners around `pivot`, in the mesh's own space. The pivot is the
    /// selection's centre: turning about a point the face does not contain swings it away.
    pub fn turn_corners(&mut self, corners: &[u32], pivot: Vec3, by: Quat) {
        for corner in corners {
            if let Some(position) = self.positions.get_mut(*corner as usize) {
                *position = pivot + by * (*position - pivot);
            }
        }
    }

    /// Scales the given corners about `pivot`, per axis. Clamped away from zero, or a collapsed
    /// corner could never be dragged back out.
    pub fn scale_corners(&mut self, corners: &[u32], pivot: Vec3, by: Vec3) {
        let by = by.max(Vec3::splat(0.001));
        for corner in corners {
            if let Some(position) = self.positions.get_mut(*corner as usize) {
                *position = pivot + (*position - pivot) * by;
            }
        }
    }

    /// Triangulates every face as a fan over the shared positions: welded for the collider, where
    /// rendering takes the split [`to_mesh`](Self::to_mesh). A fan is exact because faces are
    /// convex.
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

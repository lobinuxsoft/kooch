//! Edges, and which faces meet along them — rebuilt, never stored.

use std::collections::HashMap;

use crate::BlockMesh;

/// No face on this side of the edge.
///
/// A sentinel rather than `Option<u32>`: this is SoA, and a `u32` that
/// packs beats an enum that pads.
pub const NO_FACE: u32 = u32::MAX;

/// The edges of a [`BlockMesh`] and the faces along each.
///
/// # Derived, never serialised
///
/// `BlockMesh` stores positions and the faces that index them, and
/// nothing else. Everything here is implied by those, so writing it to
/// the file would put a second copy of one truth in every saved level —
/// and the two would disagree the first time an operator updated one and
/// forgot the other.
///
/// The cost of rebuilding is one pass over the face-corners. The cost of
/// being wrong is a level that loads with holes in it.
///
/// It also means this layout is free to change: no saved level depends
/// on any of it.
///
/// # Why there are no "wings"
///
/// A winged-edge mesh stores, per edge, the next edge clockwise around
/// each of its two faces — `godot-ply`'s `edge_edges`. It needs them
/// because its faces are **implicit**: a face is one starting edge, so
/// enumerating it means walking the wings.
///
/// `BlockMesh` stores its faces **explicitly**, as a contiguous run of
/// corners. Enumerating one is a slice. So [`face_edges`] is aligned
/// one-to-one with `BlockMesh::face_corners` — entry `j` is the edge
/// leaving corner `j` within its own face — and the walk, the sides and
/// the four wings per edge all disappear.
///
/// [`face_edges`]: Adjacency::face_edges
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Adjacency {
    /// Two per edge: the corners it joins, lower index first, so both
    /// faces sharing it arrive at the same key.
    edge_corners: Vec<u32>,
    /// Two per edge: the faces along it, [`NO_FACE`] where there is
    /// none. A boundary edge has one; an interior edge has two.
    edge_faces: Vec<u32>,
    /// One per face-corner, aligned with `BlockMesh::face_corners`: the
    /// edge leaving that corner, within that face.
    face_edges: Vec<u32>,
    /// Where each face begins in `face_edges`, copied from the mesh so
    /// this answers about faces without being handed one back.
    face_starts: Vec<u32>,
    /// CSR over corners: every edge meeting at each.
    corner_edges: Vec<u32>,
    corner_starts: Vec<u32>,
    /// Edges a third face tried to claim. Zero on a manifold mesh.
    crowded: u32,
}

impl Adjacency {
    /// Derives the adjacency of `mesh`.
    ///
    /// The map is build-time only and thrown away here — a corner pair
    /// has to find the edge some earlier face already made for it, and
    /// that lookup is not a hot path: this runs when an edit changes the
    /// mesh, not per frame.
    pub fn of(mesh: &BlockMesh) -> Self {
        let mut edges: HashMap<[u32; 2], u32> = HashMap::new();
        let mut edge_corners: Vec<u32> = Vec::new();
        let mut edge_faces: Vec<u32> = Vec::new();
        let mut face_edges: Vec<u32> = Vec::new();
        let mut crowded = 0;

        for face in 0..mesh.face_count() {
            let Some(corners) = mesh.face(face) else {
                continue;
            };
            for step in 0..corners.len() {
                let from = corners[step];
                let to = corners[(step + 1) % corners.len()];
                let key = match from <= to {
                    true => [from, to],
                    false => [to, from],
                };

                let edge = *edges.entry(key).or_insert_with(|| {
                    let index = (edge_corners.len() / 2) as u32;
                    edge_corners.extend_from_slice(&key);
                    edge_faces.extend_from_slice(&[NO_FACE, NO_FACE]);
                    index
                });
                face_edges.push(edge);

                // First free side. A third claimant is non-manifold and
                // is counted rather than overwriting one of the two —
                // silently keeping the last pair is how a fan becomes a
                // mesh that looks fine and extrudes wrong.
                let sides = &mut edge_faces[edge as usize * 2..edge as usize * 2 + 2];
                match sides.iter().position(|slot| *slot == NO_FACE) {
                    Some(free) => sides[free] = face as u32,
                    None => crowded += 1,
                }
            }
        }

        let corner_starts_and_edges = corner_csr(mesh.positions().len(), &edge_corners);
        Self {
            edge_corners,
            edge_faces,
            face_edges,
            face_starts: face_starts_of(mesh),
            corner_edges: corner_starts_and_edges.1,
            corner_starts: corner_starts_and_edges.0,
            crowded,
        }
    }

    /// How many distinct edges the mesh has.
    pub fn edge_count(&self) -> usize {
        self.edge_corners.len() / 2
    }

    /// The two corners `edge` joins, lower index first.
    pub fn edge_corners(&self, edge: u32) -> Option<[u32; 2]> {
        let pair = self
            .edge_corners
            .get(edge as usize * 2..edge as usize * 2 + 2)?;
        Some([pair[0], pair[1]])
    }

    /// The faces along `edge`, [`NO_FACE`] where there is none.
    pub fn edge_faces(&self, edge: u32) -> Option<[u32; 2]> {
        let pair = self
            .edge_faces
            .get(edge as usize * 2..edge as usize * 2 + 2)?;
        Some([pair[0], pair[1]])
    }

    /// The faces actually along `edge` — one for a boundary, two for an
    /// interior edge.
    pub fn faces_of(&self, edge: u32) -> impl Iterator<Item = u32> + '_ {
        self.edge_faces(edge)
            .unwrap_or([NO_FACE, NO_FACE])
            .into_iter()
            .filter(|face| *face != NO_FACE)
    }

    /// The edges around `face`, in its own winding order. Entry `k` is
    /// the edge leaving that face's corner `k`.
    pub fn edges_of(&self, face: usize) -> Option<&[u32]> {
        let start = *self.face_starts.get(face)? as usize;
        let end = *self.face_starts.get(face + 1)? as usize;
        self.face_edges.get(start..end)
    }

    /// Every edge meeting at `corner`.
    pub fn edges_at(&self, corner: u32) -> Option<&[u32]> {
        let start = *self.corner_starts.get(corner as usize)? as usize;
        let end = *self.corner_starts.get(corner as usize + 1)? as usize;
        self.corner_edges.get(start..end)
    }

    /// Whether `edge` has a face on one side only.
    pub fn is_boundary(&self, edge: u32) -> bool {
        self.faces_of(edge).count() == 1
    }

    /// Whether no edge is claimed by a third face.
    ///
    /// A boundary edge is manifold: an open quad is a legitimate mesh.
    /// What is not is three faces meeting along one edge, which has no
    /// consistent normal and no boundary an operator can walk.
    pub fn is_manifold(&self) -> bool {
        self.crowded == 0
    }

    /// Whether every edge has two faces — the mesh encloses a volume.
    ///
    /// What a collider wants to hear. An open mesh still renders and
    /// still extrudes; a body built from one lets things through the
    /// hole.
    pub fn is_closed(&self) -> bool {
        self.is_manifold() && (0..self.edge_count() as u32).all(|edge| !self.is_boundary(edge))
    }
}

/// The mesh's face offsets, so this can answer about faces on its own.
fn face_starts_of(mesh: &BlockMesh) -> Vec<u32> {
    let mut starts = Vec::with_capacity(mesh.face_count() + 1);
    let mut at = 0u32;
    for face in mesh.faces() {
        starts.push(at);
        at += face.len() as u32;
    }
    if mesh.face_count() > 0 {
        starts.push(at);
    }
    starts
}

/// Counting sort of edges by corner: offsets first, then a second pass
/// that fills. Two passes and no per-corner allocation.
fn corner_csr(corners: usize, edge_corners: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let mut starts = vec![0u32; corners + 1];
    for corner in edge_corners {
        starts[*corner as usize + 1] += 1;
    }
    for index in 1..starts.len() {
        starts[index] += starts[index - 1];
    }

    let mut filled = starts.clone();
    let mut edges = vec![0u32; edge_corners.len()];
    for (slot, corner) in edge_corners.iter().enumerate() {
        let edge = (slot / 2) as u32;
        let at = &mut filled[*corner as usize];
        edges[*at as usize] = edge;
        *at += 1;
    }
    (starts, edges)
}

#[cfg(test)]
mod tests;

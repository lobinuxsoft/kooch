//! Pulling faces out and stitching walls behind them.

use std::collections::HashMap;

use glam::Vec3;

use crate::{Adjacency, BlockMesh};

/// What an extrude produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Extruded {
    /// The faces to keep selected, so a second extrude continues the
    /// wall rather than starting beside it.
    pub faces: Vec<u32>,
    /// The side walls it stitched, in the order their edges were found.
    pub walls: Vec<u32>,
}

impl BlockMesh {
    /// Pulls `faces` along `by` and stitches walls to the boundary.
    ///
    /// # Contiguous faces move as one piece
    ///
    /// Extruding one face is easy. The real case is several — a 2×3
    /// patch of floor pulled up is **one wall, not six pillars** — and
    /// that is what the boundary is for: an edge used twice inside the
    /// selection is interior and gets no wall; used once, it is the rim.
    ///
    /// A selection in two disconnected pieces extrudes as two pieces in
    /// one call, because each has its own rim.
    ///
    /// # Which corners are duplicated
    ///
    /// Only the ones the surface has to keep: a corner on the rim, or
    /// one an unselected face still uses. A corner entirely inside the
    /// patch is simply moved — duplicating it would leave the original
    /// belonging to nothing, which is a hole the renderer draws through.
    ///
    /// # Winding
    ///
    /// A wall is `[a, b, b', a']` for a rim edge `a → b` taken in its
    /// own face's order, which puts its normal away from the volume.
    /// Not derived from a normal: a face dragged flat has no reliable
    /// one, and the winding is already known.
    ///
    /// Answers `None` when `faces` names nothing that exists.
    pub fn extrude(&mut self, faces: &[u32], by: Vec3) -> Option<Extruded> {
        let selected: Vec<u32> = faces
            .iter()
            .copied()
            .filter(|face| self.face(*face as usize).is_some())
            .collect();
        if selected.is_empty() {
            return None;
        }

        let adjacency = Adjacency::of(self);
        let rim = self.rim_edges(&adjacency, &selected);
        let split = self.corners_to_split(&adjacency, &selected, &rim);

        // The duplicates are the ones that travel; the originals stay
        // put for the faces around them.
        let mut moved: HashMap<u32, u32> = HashMap::new();
        for corner in &split {
            let at = self.positions[*corner as usize] + by;
            moved.insert(*corner, self.push_position(at));
        }

        let mut walls = Vec::with_capacity(rim.len());
        for (face, step) in &rim {
            let corners = self.face(*face as usize)?.to_vec();
            let a = corners[*step];
            let b = corners[(step + 1) % corners.len()];
            // Both ends are split by construction — `corners_to_split`
            // takes every rim endpoint — so the wall is never degenerate.
            let (top_a, top_b) = (moved[&a], moved[&b]);
            walls.push(self.push_face(&[a, b, top_b, top_a]));
        }

        // 🔴 Read before the faces are rewritten. Afterwards
        // `corners_of` answers with the COPIES, which already carry
        // `by`, and moving those again sends the face twice as far.
        let interior: Vec<u32> = self
            .corners_of(&selected)
            .into_iter()
            .filter(|corner| !moved.contains_key(corner))
            .collect();

        // Rewritten after the walls, which read the faces as they were.
        for face in &selected {
            self.retarget_face(*face, &moved);
        }
        // A corner nothing else uses has no copy, so it just travels.
        for corner in interior {
            self.positions[corner as usize] += by;
        }

        Some(Extruded {
            faces: selected,
            walls,
        })
    }

    /// The direction a selection extrudes along by default: the
    /// average of its faces' normals, times `distance`.
    ///
    /// 🔴 Averaged, not per face. Extruding a curved patch along each
    /// face's own normal tears it into a fan of disconnected pieces —
    /// the faces diverge, and the walls between them have nowhere to
    /// meet. One direction keeps the patch a patch.
    ///
    /// `None` when nothing is selected, or when the normals cancel:
    /// two opposite faces have no shared "out", and picking one of them
    /// would extrude half the selection backwards.
    pub fn extrude_direction(&self, faces: &[u32], distance: f32) -> Option<Vec3> {
        let mut total = Vec3::ZERO;
        let mut counted = 0;
        for face in faces {
            if let Some(normal) = self.face_normal(*face as usize) {
                total += normal;
                counted += 1;
            }
        }
        if counted == 0 {
            return None;
        }
        let average = total / counted as f32;
        // Not `normalize_or`: a fallback direction here would be a
        // guess about which way the author meant, and there is no
        // answer for a selection that faces both ways.
        (average.length() > 1e-4).then(|| average.normalize() * distance)
    }

    /// The rim of the selection: `(face, step)` for every edge used by
    /// exactly one selected face.
    ///
    /// Named by the face and the step inside it rather than by the edge,
    /// because the wall's winding comes from that face's order and an
    /// edge index has forgotten it.
    fn rim_edges(&self, adjacency: &Adjacency, selected: &[u32]) -> Vec<(u32, usize)> {
        let mut uses: HashMap<u32, u32> = HashMap::new();
        for face in selected {
            let Some(edges) = adjacency.edges_of(*face as usize) else {
                continue;
            };
            for edge in edges {
                *uses.entry(*edge).or_default() += 1;
            }
        }

        let mut rim = Vec::new();
        for face in selected {
            let Some(edges) = adjacency.edges_of(*face as usize) else {
                continue;
            };
            for (step, edge) in edges.iter().enumerate() {
                if uses.get(edge) == Some(&1) {
                    rim.push((*face, step));
                }
            }
        }
        rim
    }

    /// Corners that need a copy left behind.
    ///
    /// A rim endpoint always, because the wall is stitched between the
    /// old and the new. And any corner an unselected face still uses,
    /// or moving it would drag that face along.
    fn corners_to_split(
        &self,
        adjacency: &Adjacency,
        selected: &[u32],
        rim: &[(u32, usize)],
    ) -> Vec<u32> {
        let mut split: Vec<u32> = Vec::new();
        let mut add = |corner: u32, split: &mut Vec<u32>| {
            if !split.contains(&corner) {
                split.push(corner);
            }
        };

        for (face, step) in rim {
            let Some(corners) = self.face(*face as usize) else {
                continue;
            };
            add(corners[*step], &mut split);
            add(corners[(step + 1) % corners.len()], &mut split);
        }

        for corner in self.corners_of(selected) {
            let touched_by_others = adjacency
                .edges_at(corner)
                .unwrap_or_default()
                .iter()
                .flat_map(|edge| adjacency.faces_of(*edge))
                .any(|face| !selected.contains(&face));
            if touched_by_others {
                add(corner, &mut split);
            }
        }
        split
    }

    /// Appends a position and answers its index.
    fn push_position(&mut self, at: Vec3) -> u32 {
        self.positions.push(at);
        (self.positions.len() - 1) as u32
    }

    /// Appends a face and answers its index.
    fn push_face(&mut self, corners: &[u32]) -> u32 {
        if self.face_starts.is_empty() {
            self.face_starts.push(0);
        }
        self.face_corners.extend_from_slice(corners);
        self.face_starts.push(self.face_corners.len() as u32);
        (self.face_starts.len() - 2) as u32
    }

    /// Points a face's corners at their copies, where one was made.
    fn retarget_face(&mut self, face: u32, moved: &HashMap<u32, u32>) {
        let Some(start) = self.face_starts.get(face as usize).copied() else {
            return;
        };
        let Some(end) = self.face_starts.get(face as usize + 1).copied() else {
            return;
        };
        for slot in start as usize..end as usize {
            if let Some(to) = moved.get(&self.face_corners[slot]) {
                self.face_corners[slot] = *to;
            }
        }
    }
}

#[cfg(test)]
mod tests;

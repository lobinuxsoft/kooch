//! [`ColliderMeshCache`]: plain vertices for mesh-derived colliders, defined here and filled by a
//! crate that can see meshes — resolving a [`Guid`] would tie
//! [`PhysicsBackend`](super::PhysicsBackend) to wgpu. Testable by inserting triangles by hand.

use std::collections::HashMap;

use glam::Vec3;
use kooch_core::Guid;

use super::MeshKey;

use super::shape::ConvexPart;

/// A mesh as physics sees it, with reductions already paid for: a 387-point `hull` against 76 038
/// `vertices`, so rebuilds on every scale drag don't rerun qhull or clone the large set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColliderMesh {
    pub vertices: Vec<Vec3>,
    /// Triangles as indices into `vertices`; empty for a cloud that only feeds a hull.
    pub indices: Vec<[u32; 3]>,
    /// The convex hull with its faces, or empty when nobody asked — computed on demand.
    pub hull: ConvexPart,
    /// Baked convex pieces from a `.glb`, one primitive each — their presence skips VHACD, seconds
    /// instead of milliseconds.
    pub parts: Vec<ConvexPart>,
}

impl ColliderMesh {
    /// A mesh with no triangles worth colliding against.
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty() && self.parts.is_empty()
    }

    /// The reduced hull when it exists, else the raw cloud — the same hull, dearer, and the next
    /// rebuild gets cheaper.
    pub fn hull_or_vertices(&self) -> ConvexPart {
        match self.hull.is_empty() {
            true => ConvexPart::loose(self.vertices.clone()),
            false => self.hull.clone(),
        }
    }
}

/// What a mesh-derived collider is waiting for.
#[derive(Debug, Clone, PartialEq)]
enum Entry {
    /// The loader tried and could not. Kept rather than dropped so the
    /// difference between "not yet" and "never" survives.
    Failed,
    Ready(ColliderMesh),
}

/// Mesh data for colliders, keyed by asset GUID.
#[derive(Debug, Default)]
pub struct ColliderMeshCache {
    entries: HashMap<MeshKey, (u64, Entry)>,
    /// Monotonic, never reset. What a body's spec carries so that a mesh
    /// arriving *after* the body was authored rebuilds it — the spec
    /// compares unequal the moment this moves.
    next_epoch: u64,
}

impl ColliderMeshCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publishes a mesh, replacing whatever was there.
    pub fn insert(&mut self, key: impl Into<MeshKey>, mesh: ColliderMesh) {
        let guid = key.into();
        self.next_epoch += 1;
        self.entries
            .insert(guid, (self.next_epoch, Entry::Ready(mesh)));
    }

    /// Publishes a reduced hull, bumping the epoch so bodies built from the full cloud rebuild.
    pub fn insert_hull(&mut self, key: impl Into<MeshKey>, hull: ConvexPart) {
        let guid = key.into();
        let Some((epoch, Entry::Ready(mesh))) = self.entries.get_mut(&guid) else {
            return;
        };
        mesh.hull = hull;
        self.next_epoch += 1;
        *epoch = self.next_epoch;
    }

    /// `true` when this GUID has a mesh whose hull has not been reduced.
    pub fn awaits_hull(&self, key: impl Into<MeshKey>) -> bool {
        let guid = key.into();
        matches!(self.entries.get(&guid), Some((_, Entry::Ready(mesh))) if mesh.hull.is_empty())
    }

    /// Records a GUID that will not resolve. Idempotent: the filler runs every frame and must not
    /// rebuild every body.
    pub fn fail(&mut self, key: impl Into<MeshKey>) {
        let guid = key.into();
        if matches!(self.entries.get(&guid), Some((_, Entry::Failed))) {
            return;
        }
        self.next_epoch += 1;
        self.entries.insert(guid, (self.next_epoch, Entry::Failed));
    }

    /// The mesh, or `None` while it is missing or broken.
    pub fn get(&self, key: impl Into<MeshKey>) -> Option<&ColliderMesh> {
        let guid = key.into();
        match self.entries.get(&guid) {
            Some((_, Entry::Ready(mesh))) => Some(mesh),
            _ => None,
        }
    }

    /// How often this GUID's answer changed; `0` means unanswered, distinguishing it in a
    /// [`ShapeSpec`](crate::components::ShapeSpec).
    pub fn epoch(&self, key: impl Into<MeshKey>) -> u64 {
        let guid = key.into();
        self.entries
            .get(&guid)
            .map(|(epoch, _)| *epoch)
            .unwrap_or(0)
    }

    /// `true` once something has answered for this GUID, either way.
    pub fn answered(&self, key: impl Into<MeshKey>) -> bool {
        let guid = key.into();
        self.entries.contains_key(&guid)
    }

    /// How many GUIDs have an answer.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drops every entry but keeps the epoch counting, or a refilled GUID never rebuilds its
    /// bodies.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests;

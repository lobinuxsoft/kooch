//! [`ColliderMeshCache`] — the vertices a mesh-derived collider is built
//! from, and nothing about where they came from.
//!
//! # The bridge points this way on purpose
//!
//! A collider authored as "use that mesh" names a [`Guid`]. Resolving one
//! means an asset database, which means the renderer — and physics
//! depending on the renderer would tie [`PhysicsBackend`] to wgpu, which
//! is the one thing the trait exists to avoid.
//!
//! So the cache is *defined* here and *filled* from outside, by a system
//! in a crate that can already see meshes. Physics reads plain points and
//! never asks who put them there. That also makes it testable without an
//! asset server: insert the triangles by hand.
//!
//! [`PhysicsBackend`]: super::PhysicsBackend

use std::collections::HashMap;

use glam::Vec3;
use kooch_core::Guid;

/// A mesh, as physics sees it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColliderMesh {
    pub vertices: Vec<Vec3>,
    /// Triangles, as indices into `vertices`.
    ///
    /// Empty for a point cloud that only ever feeds a convex hull, which
    /// needs no topology.
    pub indices: Vec<[u32; 3]>,
}

impl ColliderMesh {
    /// A mesh with no triangles worth colliding against.
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
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
    entries: HashMap<Guid, (u64, Entry)>,
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
    pub fn insert(&mut self, guid: Guid, mesh: ColliderMesh) {
        self.next_epoch += 1;
        self.entries
            .insert(guid, (self.next_epoch, Entry::Ready(mesh)));
    }

    /// Records that this GUID will not resolve.
    ///
    /// Idempotent by design: the filler runs every frame and must not
    /// bump the epoch — and so rebuild every body — for a failure that
    /// has not changed.
    pub fn fail(&mut self, guid: Guid) {
        if matches!(self.entries.get(&guid), Some((_, Entry::Failed))) {
            return;
        }
        self.next_epoch += 1;
        self.entries.insert(guid, (self.next_epoch, Entry::Failed));
    }

    /// The mesh, or `None` while it is missing or broken.
    pub fn get(&self, guid: Guid) -> Option<&ColliderMesh> {
        match self.entries.get(&guid) {
            Some((_, Entry::Ready(mesh))) => Some(mesh),
            _ => None,
        }
    }

    /// How many times this GUID's answer has changed, engine-wide.
    ///
    /// `0` for a GUID nobody has answered for yet, which is what makes an
    /// unresolved collider distinguishable from a resolved one in a
    /// [`ShapeSpec`](crate::components::ShapeSpec).
    pub fn epoch(&self, guid: Guid) -> u64 {
        self.entries
            .get(&guid)
            .map(|(epoch, _)| *epoch)
            .unwrap_or(0)
    }

    /// `true` once something has answered for this GUID, either way.
    pub fn answered(&self, guid: Guid) -> bool {
        self.entries.contains_key(&guid)
    }

    /// How many GUIDs have an answer.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drops every entry.
    ///
    /// The epoch deliberately keeps counting: a GUID cleared and
    /// refilled with the same mesh must still read as a change, or a body
    /// built from the old data never rebuilds.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests;

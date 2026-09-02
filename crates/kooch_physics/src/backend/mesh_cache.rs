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

/// A mesh, as physics sees it — with whatever reductions of it have
/// already been paid for.
///
/// # Why the reductions live here
///
/// `hull` is 387 points where `vertices` is 76 038, and a body's shape is
/// rebuilt whenever its spec changes — a scale drag, a friction edit.
/// Deriving the hull each time means qhull over the large set every
/// rebuild, and scaling it means cloning the large set every frame.
/// Reduced once, both become the small set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColliderMesh {
    pub vertices: Vec<Vec3>,
    /// Triangles, as indices into `vertices`.
    ///
    /// Empty for a point cloud that only ever feeds a convex hull, which
    /// needs no topology.
    pub indices: Vec<[u32; 3]>,
    /// The convex hull of `vertices`, or empty when nobody has asked.
    ///
    /// Computed on demand: a collider that only ever wants the triangles
    /// should not pay for a hull it will not use.
    pub hull: Vec<Vec3>,
    /// Convex pieces, when the source was authored as several.
    ///
    /// Non-empty only for a **baked** decomposition — a `.glb` holding
    /// one primitive per piece. Its presence is what lets a concave
    /// collider skip VHACD, which is seconds rather than milliseconds.
    pub parts: Vec<Vec<Vec3>>,
}

impl ColliderMesh {
    /// A mesh with no triangles worth colliding against.
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty() && self.parts.is_empty()
    }

    /// The points a convex hull should be built from: the reduced set
    /// when it exists, the full one until it does.
    ///
    /// Falling back rather than waiting — the hull of the full cloud is
    /// the same hull, just dearer, so a body built the frame before the
    /// reduction lands is correct and gets cheaper on its next rebuild.
    pub fn hull_or_vertices(&self) -> &[Vec3] {
        match self.hull.is_empty() {
            true => &self.vertices,
            false => &self.hull,
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

    /// Publishes the reduced hull for a mesh already in the cache.
    ///
    /// Bumps the epoch like any other answer, so a body built from the
    /// full cloud is retired and rebuilt from the small one.
    pub fn insert_hull(&mut self, guid: Guid, hull: Vec<Vec3>) {
        let Some((epoch, Entry::Ready(mesh))) = self.entries.get_mut(&guid) else {
            return;
        };
        mesh.hull = hull;
        self.next_epoch += 1;
        *epoch = self.next_epoch;
    }

    /// `true` when this GUID has a mesh whose hull has not been reduced.
    pub fn awaits_hull(&self, guid: Guid) -> bool {
        matches!(self.entries.get(&guid), Some((_, Entry::Ready(mesh))) if mesh.hull.is_empty())
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

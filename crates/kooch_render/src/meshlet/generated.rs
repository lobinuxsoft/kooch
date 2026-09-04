//! Meshes built at runtime rather than loaded from disk.

use std::collections::HashMap;

use kooch_core::Guid;

use super::asset::MeshletMesh;

/// Meshes some other crate generated, waiting to reach the GPU pool.
///
/// The asset path resolves a `MeshRenderer.mesh` GUID by asking the
/// `AssetServer` for a file. A mesh built this frame — a block being
/// extruded, a procedural shape — has no file, and writing one to disk
/// just to read it back is a round trip through the filesystem per
/// keystroke.
///
/// So a generator publishes here instead, keyed by the same GUID its
/// `MeshRenderer` points at, and the meshlet sync drains this before it
/// goes looking on disk. Nothing in this crate needs to know what a
/// block is.
#[derive(Debug, Default)]
pub struct GeneratedMeshes {
    pending: HashMap<Guid, MeshletMesh>,
}

impl GeneratedMeshes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publishes a mesh under `guid`, replacing whatever was waiting
    /// there. Replacing rather than queueing on purpose: an edit in
    /// flight supersedes the one before it, and the intermediate states
    /// of a drag are not worth uploading.
    pub fn insert(&mut self, guid: Guid, mesh: MeshletMesh) {
        self.pending.insert(guid, mesh);
    }

    /// Takes everything published so far, leaving the store empty.
    pub fn drain(&mut self) -> Vec<(Guid, MeshletMesh)> {
        self.pending.drain().collect()
    }

    /// Whether anything is waiting.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// How many meshes are waiting.
    pub fn len(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests;

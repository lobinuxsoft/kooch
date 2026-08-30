//! What the project remembers about where everything was last frame.
//!
//! # Why this is not [`SnapshotCache`](crate::snapshot_cache::SnapshotCache)
//!
//! That one answers *what is the world*, and to answer it the host
//! reflects every field of every component of every entity into strings
//! before it diffs. Measured on `dense.scene`: **38.9 ms of the editor's
//! 46 ms frame**, waiting, on 2159 entities — and then throwing almost
//! all of it away, because a cube that did not move describes itself
//! identically.
//!
//! This answers *what moved*, which is a direct read of one component
//! and a compare of sixteen floats. Nothing is reflected and nothing is
//! allocated per component.
//!
//! # When the host refuses
//!
//! A transform diff only describes a world the caller already has. When
//! the entity SET changes — a spawn, a despawn — the reply is `full` and
//! carries nothing: the caller is being told to ask the other question
//! instead. Answering with transforms anyway would leave the editor
//! drawing a world missing whatever was created.

use std::collections::HashMap;

use crate::protocol::{EntityId, MovedTransform};

/// The last transform sent for each entity, and the revision that
/// described it.
#[derive(Default)]
pub struct MovedCache {
    last: HashMap<EntityId, [f32; 16]>,
    revision: u64,
}

/// What one [`MovedCache::reply`] produced.
pub struct MovedDelta {
    pub moved: Vec<MovedTransform>,
    pub removed: Vec<EntityId>,
    pub revision: u64,
    pub full: bool,
}

impl MovedCache {
    /// Diffs `current` against the last world described.
    ///
    /// `full` when the caller's revision is not the one this holds, or
    /// when an entity appeared — both mean the caller's world and this
    /// one disagree about more than positions.
    pub fn reply(&mut self, current: Vec<MovedTransform>, since: Option<u64>) -> MovedDelta {
        let appeared = current.iter().any(|m| !self.last.contains_key(&m.id));
        let stale = since != Some(self.revision);

        let removed: Vec<EntityId> = if appeared || stale {
            Vec::new()
        } else {
            let present: std::collections::HashSet<EntityId> =
                current.iter().map(|m| m.id).collect();
            self.last
                .keys()
                .copied()
                .filter(|id| !present.contains(id))
                .collect()
        };

        let moved: Vec<MovedTransform> = if appeared || stale {
            Vec::new()
        } else {
            current
                .iter()
                .copied()
                .filter(|m| self.last.get(&m.id) != Some(&m.matrix))
                .collect()
        };

        // 🔴 The revision moves only when the reply does. A bump on a
        // frame that said nothing would leave the caller holding a
        // revision for a world it was never sent, and the next diff
        // would be computed against it.
        if !moved.is_empty() || !removed.is_empty() || appeared || stale {
            self.revision += 1;
        }
        self.last = current.into_iter().map(|m| (m.id, m.matrix)).collect();

        MovedDelta {
            moved,
            removed,
            revision: self.revision,
            full: appeared || stale,
        }
    }
}

#[cfg(test)]
mod tests;

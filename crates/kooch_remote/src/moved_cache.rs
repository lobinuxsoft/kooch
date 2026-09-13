//! Where everything was last frame: one component and sixteen floats, not the 38.9 ms reflection of
//! [`SnapshotCache`](crate::snapshot_cache::SnapshotCache). A changed entity set replies `full` and
//! empty — ask the other question.

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
    /// Diffs `current` against the last world described; `full` on a stale revision or a new
    /// entity, where the worlds differ beyond positions.
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

        // 🔴 The revision moves only with a reply, or the caller holds a revision for a world it was
        // never sent.
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

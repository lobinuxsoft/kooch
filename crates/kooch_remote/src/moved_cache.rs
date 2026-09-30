//! Where everything was last frame: one component and sixteen floats, not the 38.9 ms reflection of
//! [`SnapshotCache`](crate::snapshot_cache::SnapshotCache). A changed entity set replies `full` and
//! empty — ask the other question.

use std::collections::HashMap;

use kooch_ecs::reflect::ReflectValue;

use crate::protocol::{EntityId, MovedComponent, MovedTransform};

/// The last transform sent for each entity, and the revision that
/// described it.
#[derive(Default)]
pub struct MovedCache {
    last: HashMap<EntityId, [f32; 16]>,
    /// The last value sent for each declared component, keyed by entity and type name (#1407).
    last_components: HashMap<(EntityId, String), Vec<(String, ReflectValue)>>,
    revision: u64,
}

/// What one [`MovedCache::reply`] produced.
pub struct MovedDelta {
    pub moved: Vec<MovedTransform>,
    pub components: Vec<MovedComponent>,
    pub removed: Vec<EntityId>,
    pub revision: u64,
    pub full: bool,
}

impl MovedCache {
    /// Diffs `current` against the last world described; `full` on a stale revision or a new
    /// entity, where the worlds differ beyond positions.
    pub fn reply(&mut self, current: Vec<MovedTransform>, since: Option<u64>) -> MovedDelta {
        self.reply_with(current, Vec::new(), since)
    }

    /// The same, with the declared components this frame (#1407).
    pub fn reply_with(
        &mut self,
        current: Vec<MovedTransform>,
        components: Vec<MovedComponent>,
        since: Option<u64>,
    ) -> MovedDelta {
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

        // 🔴 Diffed like the transforms and for the same reason: an unchanged value costs a
        // comparison, not a message. On a stale revision the caller is pulling the whole world
        // anyway, so sending these would be work it throws away.
        let changed: Vec<MovedComponent> = if appeared || stale {
            Vec::new()
        } else {
            components
                .iter()
                .filter(|entry| {
                    let key = (entry.id, entry.component.type_name.clone());
                    self.last_components.get(&key) != Some(&entry.component.fields)
                })
                .cloned()
                .collect()
        };

        // 🔴 The revision moves only with a reply, or the caller holds a revision for a world it was
        // never sent.
        if !moved.is_empty() || !removed.is_empty() || !changed.is_empty() || appeared || stale {
            self.revision += 1;
        }
        self.last = current.into_iter().map(|m| (m.id, m.matrix)).collect();
        self.last_components = components
            .into_iter()
            .map(|entry| {
                (
                    (entry.id, entry.component.type_name),
                    entry.component.fields,
                )
            })
            .collect();

        MovedDelta {
            moved,
            components: changed,
            removed,
            revision: self.revision,
            full: appeared || stale,
        }
    }
}

#[cfg(test)]
mod tests;

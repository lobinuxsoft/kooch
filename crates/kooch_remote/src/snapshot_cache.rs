//! What the project remembers about the last world it described, so an unchanged scene costs
//! nothing to send (#691: 424.6 KB and ~32 ms a frame before).
//! Compared, not dirty-tracked, so it cannot miss a change.

use std::collections::HashMap;

use crate::protocol::{EntityId, EntitySnapshot};

/// The last world state handed out, and the revision that named it.
#[derive(Default)]
pub struct SnapshotCache {
    /// Incremented on every reply. A client passes back the revision it
    /// holds; only the most recent one can be diffed against, because
    /// only the most recent world is remembered.
    revision: u64,
    /// The entities as last sent, by id.
    last: HashMap<EntityId, EntitySnapshot>,
}

/// One reply's worth of world: what changed, what vanished, and whether
/// this is the whole thing.
pub struct SnapshotDelta {
    pub entities: Vec<EntitySnapshot>,
    pub removed: Vec<EntityId>,
    pub revision: u64,
    pub full: bool,
}

impl SnapshotCache {
    /// Turns a freshly built world into a reply for a caller holding `since` — full whenever that
    /// is not the revision this cache last issued.
    pub fn reply(&mut self, world: Vec<EntitySnapshot>, since: Option<u64>) -> SnapshotDelta {
        let can_diff = since == Some(self.revision) && self.revision > 0;
        self.revision = self.revision.wrapping_add(1);
        // Zero means "no revision yet", so a wrap has to skip it or the
        // client after 2^64 replies would be told it can diff against a
        // cache that was just reset.
        if self.revision == 0 {
            self.revision = 1;
        }

        if !can_diff {
            self.last = world.iter().map(|e| (e.id, e.clone())).collect();
            return SnapshotDelta {
                entities: world,
                removed: Vec::new(),
                revision: self.revision,
                full: true,
            };
        }

        let mut changed = Vec::new();
        let mut next = HashMap::with_capacity(world.len());
        for entity in world {
            // `!=` on the whole snapshot: a component added, a field
            // edited and a name changed are all the same question, and
            // asking it once means none of them can be forgotten.
            let differs = self.last.get(&entity.id) != Some(&entity);
            next.insert(entity.id, entity.clone());
            if differs {
                changed.push(entity);
            }
        }

        // Anything the previous world had and this one lacks — or a despawned entity would live
        // forever in the mirror.
        let removed = self
            .last
            .keys()
            .filter(|id| !next.contains_key(id))
            .copied()
            .collect();

        self.last = next;
        SnapshotDelta {
            entities: changed,
            removed,
            revision: self.revision,
            full: false,
        }
    }
}

impl SnapshotCache {
    /// Component type names whose values changed since the last world this cache described, minus
    /// the ones a plugin declared as streamed (#1407).
    ///
    /// 🔴 The guard on a hand-maintained list. `StreamedComponents` is what reaches a watching
    /// editor every frame, and forgetting to declare a component there used to fail the way #1254
    /// failed: the author sees nothing and has no way to tell why. Called only while the project
    /// PLAYS, where the editor cannot edit — so every difference is the gameplay's, and there are no
    /// false positives.
    ///
    /// Free: the comparison is the one `reply` already makes, over a world already reflected.
    pub fn undeclared_changes(&self, world: &[EntitySnapshot], declared: &[&str]) -> Vec<String> {
        let mut named: Vec<String> = Vec::new();
        for entity in world {
            let Some(before) = self.last.get(&entity.id) else {
                // A new entity is not a changed component.
                continue;
            };
            for component in &entity.components {
                if declared.contains(&component.type_name.as_str()) {
                    continue;
                }
                let unchanged = before.components.iter().any(|had| {
                    had.type_name == component.type_name && had.fields == component.fields
                });
                if unchanged {
                    continue;
                }
                if !named.contains(&component.type_name) {
                    named.push(component.type_name.clone());
                }
            }
        }
        named
    }
}

#[cfg(test)]
mod tests;

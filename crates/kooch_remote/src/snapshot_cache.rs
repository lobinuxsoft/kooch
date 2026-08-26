//! What the project remembers about the last world it described.
//!
//! # Why
//!
//! The editor mirrors the project's ECS by asking for every entity, every
//! frame. Measured on a 608-instance scene: **424.6 KB per frame for 610
//! entities**, 13.7 ms of it spent parsing JSON on the editor's main
//! thread, with another 7.5 ms rebuilding the mirror — about 32 ms a
//! frame, against 0.19 ms of actual GPU work (#691).
//!
//! Almost all of it is redundant. A scene where nothing moved describes
//! itself identically every frame.
//!
//! # Comparing rather than tracking
//!
//! The obvious approach is change detection in the ECS — a dirty flag per
//! component, the way Bevy's `Changed<T>` works. That is the better
//! long-term answer and a much larger change, and it has a failure mode
//! this does not: a write path that forgets to mark dirty produces a
//! mirror that is silently stale, and the bug shows up as "the editor
//! sometimes doesn't update".
//!
//! This compares the snapshot it just built against the one it sent last
//! time. `EntitySnapshot` already derives `PartialEq`, so the comparison
//! is exact — no hashing, so no collisions to reason about — and it
//! cannot miss a change, because it looks at the value rather than
//! trusting anyone to have flagged it.
//!
//! The cost is that the project still walks its whole world each frame.
//! That walk is local CPU work in the *project's* process; what the
//! measurement showed to be expensive is the transport and the parse on
//! the editor's side, and those are what this removes.

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
    /// Turns a freshly built world into a reply for a caller holding
    /// `since`.
    ///
    /// A full reply goes out whenever the caller's revision is not the
    /// one this cache last issued — a first call, a dropped frame, a
    /// project that restarted and began counting again. Diffing against
    /// a revision that is not the immediately preceding one would be
    /// diffing against a world nobody has.
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

        // Anything the previous world had and this one does not. Without
        // this a despawned entity would live forever in the mirror,
        // because a diff that only carries changes never mentions it
        // again.
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

#[cfg(test)]
mod tests;

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
mod tests {
    use super::*;
    use crate::protocol::{ComponentSnapshot, EntityId};
    use kooch_ecs::reflect::ReflectValue;

    fn entity(index: u32, x: f32) -> EntitySnapshot {
        EntitySnapshot {
            id: EntityId {
                index,
                generation: 0,
            },
            name: Some(format!("Entity {index}")),
            parent: None,
            components: vec![ComponentSnapshot {
                type_name: "Transform".to_owned(),
                fields: vec![("x".to_owned(), ReflectValue::F32(x))],
            }],
        }
    }

    /// A client with no revision has nothing to diff against.
    #[test]
    fn the_first_reply_is_always_full() {
        let mut cache = SnapshotCache::default();
        let reply = cache.reply(vec![entity(0, 0.0), entity(1, 0.0)], None);
        assert!(reply.full);
        assert_eq!(reply.entities.len(), 2);
    }

    /// The whole point: a world that did not move sends nothing.
    #[test]
    fn an_unchanged_world_sends_no_entities() {
        let mut cache = SnapshotCache::default();
        let world = vec![entity(0, 1.0), entity(1, 2.0)];
        let first = cache.reply(world.clone(), None);

        let second = cache.reply(world, Some(first.revision));
        assert!(!second.full);
        assert!(
            second.entities.is_empty(),
            "sent {} entities for a world that did not change",
            second.entities.len(),
        );
        assert!(second.removed.is_empty());
    }

    #[test]
    fn only_the_entity_that_moved_is_sent() {
        let mut cache = SnapshotCache::default();
        let first = cache.reply(vec![entity(0, 1.0), entity(1, 2.0)], None);

        let second = cache.reply(vec![entity(0, 1.0), entity(1, 9.0)], Some(first.revision));
        assert_eq!(second.entities.len(), 1);
        assert_eq!(second.entities[0].id.index, 1);
    }

    /// A despawn is invisible to a diff of changes — it has to be named
    /// explicitly, or the mirror keeps the entity forever.
    #[test]
    fn a_despawned_entity_is_reported_as_removed() {
        let mut cache = SnapshotCache::default();
        let first = cache.reply(vec![entity(0, 1.0), entity(1, 2.0)], None);

        let second = cache.reply(vec![entity(0, 1.0)], Some(first.revision));
        assert_eq!(second.removed.len(), 1);
        assert_eq!(second.removed[0].index, 1);
        assert!(second.entities.is_empty(), "nothing changed about entity 0");
    }

    #[test]
    fn a_new_entity_is_sent() {
        let mut cache = SnapshotCache::default();
        let first = cache.reply(vec![entity(0, 1.0)], None);

        let second = cache.reply(vec![entity(0, 1.0), entity(7, 3.0)], Some(first.revision));
        assert_eq!(second.entities.len(), 1);
        assert_eq!(second.entities[0].id.index, 7);
        assert!(second.removed.is_empty());
    }

    /// A client that fell behind cannot be diffed against: the cache
    /// only remembers the most recent world. It must be told everything,
    /// and told that it was told everything.
    #[test]
    fn a_stale_revision_forces_a_full_reply() {
        let mut cache = SnapshotCache::default();
        let first = cache.reply(vec![entity(0, 1.0)], None);
        let _ = cache.reply(vec![entity(0, 2.0)], Some(first.revision));

        let stale = cache.reply(vec![entity(0, 3.0)], Some(first.revision));
        assert!(stale.full, "a stale client was handed a diff");
        assert_eq!(stale.entities.len(), 1);
    }

    /// Merging a full reply into an existing mirror would keep whatever
    /// the reply omitted, so the flag has to survive the round trip.
    #[test]
    fn a_full_reply_after_a_diff_still_says_so() {
        let mut cache = SnapshotCache::default();
        let first = cache.reply(vec![entity(0, 1.0), entity(1, 1.0)], None);
        let diff = cache.reply(vec![entity(0, 2.0), entity(1, 1.0)], Some(first.revision));
        assert!(!diff.full);

        let full = cache.reply(vec![entity(0, 2.0)], None);
        assert!(full.full);
        assert_eq!(full.entities.len(), 1);
    }
}

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

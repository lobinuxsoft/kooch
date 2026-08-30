use super::*;

fn at(id: u32, x: f32) -> MovedTransform {
    let mut matrix = [0.0; 16];
    matrix[0] = 1.0;
    matrix[5] = 1.0;
    matrix[10] = 1.0;
    matrix[12] = x;
    matrix[15] = 1.0;
    MovedTransform {
        id: EntityId {
            index: id,
            generation: 0,
        },
        matrix,
    }
}

/// The first ask cannot be a diff — there is no shared world yet.
#[test]
fn the_first_reply_is_full() {
    let mut cache = MovedCache::default();
    let reply = cache.reply(vec![at(1, 0.0), at(2, 0.0)], None);
    assert!(reply.full);
    assert!(reply.moved.is_empty(), "a full reply carries no diff");
}

/// A world that stood still costs nothing to describe.
#[test]
fn nothing_moving_sends_nothing() {
    let mut cache = MovedCache::default();
    let first = cache.reply(vec![at(1, 0.0), at(2, 0.0)], None);
    let second = cache.reply(vec![at(1, 0.0), at(2, 0.0)], Some(first.revision));
    assert!(!second.full);
    assert!(second.moved.is_empty());
    assert_eq!(
        second.revision, first.revision,
        "a reply that says nothing must not move the revision",
    );
}

/// Only the entity that moved travels.
#[test]
fn only_the_mover_travels() {
    let mut cache = MovedCache::default();
    let first = cache.reply(vec![at(1, 0.0), at(2, 0.0)], None);
    let second = cache.reply(vec![at(1, 5.0), at(2, 0.0)], Some(first.revision));
    assert_eq!(second.moved.len(), 1);
    assert_eq!(second.moved[0].id.index, 1);
}

/// 🔴 A spawn is not a movement, and answering it with transforms would
/// leave the caller drawing a world without the new entity in it.
#[test]
fn a_spawn_forces_a_full_reply() {
    let mut cache = MovedCache::default();
    let first = cache.reply(vec![at(1, 0.0)], None);
    let second = cache.reply(vec![at(1, 0.0), at(2, 0.0)], Some(first.revision));
    assert!(
        second.full,
        "an entity appeared and the reply pretended not"
    );
    assert!(second.moved.is_empty());
}

/// A despawn is describable: the caller already has the entity and is
/// being told to drop it.
#[test]
fn a_despawn_is_a_diff() {
    let mut cache = MovedCache::default();
    let first = cache.reply(vec![at(1, 0.0), at(2, 0.0)], None);
    let second = cache.reply(vec![at(1, 0.0)], Some(first.revision));
    assert!(!second.full);
    assert_eq!(second.removed.len(), 1);
    assert_eq!(second.removed[0].index, 2);
}

/// A caller holding a revision this cache never issued gets everything
/// again rather than a diff against a world nobody has.
#[test]
fn a_stale_revision_forces_a_full_reply() {
    let mut cache = MovedCache::default();
    cache.reply(vec![at(1, 0.0)], None);
    let reply = cache.reply(vec![at(1, 9.0)], Some(999));
    assert!(reply.full);
}

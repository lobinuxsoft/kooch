//! Unit tests for [`EntityAllocator`].

use super::EntityAllocator;

#[test]
fn spawn_returns_unique_entities() {
    let mut alloc = EntityAllocator::with_capacity(4);
    let a = alloc.spawn();
    let b = alloc.spawn();
    assert_ne!(a, b);
    assert_eq!(alloc.alive_count(), 2);
}

#[test]
fn sequential_indices() {
    let mut alloc = EntityAllocator::with_capacity(4);
    let a = alloc.spawn();
    let b = alloc.spawn();
    let c = alloc.spawn();
    assert_eq!(a.index(), 0);
    assert_eq!(b.index(), 1);
    assert_eq!(c.index(), 2);
}

#[test]
fn despawn_increments_generation() {
    // Use capacity 1 so the recycled slot is the only option.
    let mut alloc = EntityAllocator::with_capacity(1);
    let e = alloc.spawn();
    assert_eq!(e.generation(), 0);

    assert!(alloc.despawn(e));
    assert_eq!(alloc.alive_count(), 0);

    // Respawn the same slot — generation should be 1.
    let e2 = alloc.spawn();
    assert_eq!(e2.index(), e.index());
    assert_eq!(e2.generation(), 1);
}

#[test]
fn stale_ref_detection() {
    let mut alloc = EntityAllocator::with_capacity(4);
    let e = alloc.spawn();

    alloc.despawn(e);

    // Old handle is stale.
    assert!(!alloc.is_alive(e));
    // Despawning again returns false.
    assert!(!alloc.despawn(e));
}

#[test]
fn recycles_slots_fifo() {
    let mut alloc = EntityAllocator::with_capacity(4);
    let a = alloc.spawn(); // idx 0
    let b = alloc.spawn(); // idx 1

    alloc.despawn(a); // free 0
    alloc.despawn(b); // free 1

    // FIFO: unused slot 2 comes first, then 3, then recycled 0, 1.
    let c = alloc.spawn();
    assert_eq!(c.index(), 2);
    let d = alloc.spawn();
    assert_eq!(d.index(), 3);
    let e = alloc.spawn();
    assert_eq!(e.index(), 0);
    let f = alloc.spawn();
    assert_eq!(f.index(), 1);
}

#[test]
fn batch_spawn() {
    let mut alloc = EntityAllocator::with_capacity(8);
    let batch = alloc.spawn_batch(5);
    assert_eq!(batch.len(), 5);
    assert_eq!(alloc.alive_count(), 5);

    for (i, e) in batch.iter().enumerate() {
        assert_eq!(e.index(), i as u32);
    }
}

#[test]
fn grows_when_exhausted() {
    let mut alloc = EntityAllocator::with_capacity(2);
    let _a = alloc.spawn();
    let _b = alloc.spawn();
    // capacity exhausted — next spawn triggers grow.
    let c = alloc.spawn();
    assert_eq!(c.index(), 2);
    assert_eq!(alloc.total_slots(), 4);
}

#[test]
fn pending_sync_tracks_changes() {
    let mut alloc = EntityAllocator::with_capacity(4);
    let a = alloc.spawn();
    let b = alloc.spawn();

    let pending = alloc.take_pending_sync();
    assert_eq!(pending, vec![0, 1]);

    // Despawn produces another pending entry.
    alloc.despawn(a);
    let pending = alloc.take_pending_sync();
    assert_eq!(pending, vec![a.index()]);

    // Nothing pending after drain.
    assert!(alloc.take_pending_sync().is_empty());

    // Stale despawn does NOT add to pending.
    alloc.despawn(a);
    assert!(alloc.take_pending_sync().is_empty());

    let _ = b; // suppress unused warning
}

#[test]
fn pending_despawn_tracks_despawned_entities() {
    let mut alloc = EntityAllocator::with_capacity(4);
    let a = alloc.spawn();
    let b = alloc.spawn();

    // No despawns yet.
    assert!(alloc.take_pending_despawn().is_empty());

    alloc.despawn(a);
    let despawned = alloc.take_pending_despawn();
    assert_eq!(despawned.len(), 1);
    assert_eq!(despawned[0], a);

    // Drained — empty now.
    assert!(alloc.take_pending_despawn().is_empty());

    // Stale despawn does NOT add to pending_despawn.
    alloc.despawn(a);
    assert!(alloc.take_pending_despawn().is_empty());

    let _ = b;
}

#[test]
fn is_index_alive() {
    let mut alloc = EntityAllocator::with_capacity(4);
    let e = alloc.spawn();
    assert!(alloc.is_index_alive(e.index()));
    alloc.despawn(e);
    assert!(!alloc.is_index_alive(e.index()));
}

#[test]
fn total_slots() {
    let alloc = EntityAllocator::with_capacity(16);
    assert_eq!(alloc.total_slots(), 16);
}

#[test]
fn revive_restores_despawned_entity() {
    let mut alloc = EntityAllocator::with_capacity(4);
    let e = alloc.spawn();
    assert!(alloc.is_alive(e));

    alloc.despawn(e);
    assert!(!alloc.is_alive(e));

    assert!(alloc.revive(e));
    assert!(alloc.is_alive(e));
    assert_eq!(alloc.alive_count(), 1);
}

#[test]
fn revive_fails_if_slot_reused() {
    let mut alloc = EntityAllocator::with_capacity(1);
    let e = alloc.spawn(); // idx 0, gen 0

    alloc.despawn(e); // gen → 1
    let _e2 = alloc.spawn(); // reuses idx 0, gen 1
    alloc.despawn(_e2); // gen → 2

    // Original entity's slot has been reused, generation advanced past +1.
    assert!(!alloc.revive(e));
}

#[test]
fn revive_fails_if_still_alive() {
    let mut alloc = EntityAllocator::with_capacity(4);
    let e = alloc.spawn();

    // Entity is still alive — revive should return false.
    assert!(!alloc.revive(e));
}

#[test]
fn revive_removes_slot_from_free_list() {
    let mut alloc = EntityAllocator::with_capacity(4);
    let e = alloc.spawn(); // idx 0

    alloc.despawn(e);
    assert!(alloc.revive(e));

    // Spawn 3 more entities — should use indices 1, 2, 3 (not 0).
    let a = alloc.spawn();
    let b = alloc.spawn();
    let c = alloc.spawn();
    assert_eq!(a.index(), 1);
    assert_eq!(b.index(), 2);
    assert_eq!(c.index(), 3);
}

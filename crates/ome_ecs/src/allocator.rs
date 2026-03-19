//! Entity allocator with generational indices and GPU sync tracking.
//!
//! Uses a FIFO free-list (`VecDeque`) so recycled slots get maximum
//! temporal separation before reuse, reducing the chance of stale
//! references going undetected.

use std::collections::VecDeque;

use crate::entity::Entity;

/// Default number of pre-allocated entity slots.
const DEFAULT_CAPACITY: u32 = 1024;

/// Allocates and recycles [`Entity`] handles with generational tracking.
///
/// Every spawn/despawn is recorded in a pending-sync list so the GPU
/// `alive_mask` buffer can be updated incrementally.
pub struct EntityAllocator {
    /// Per-slot generation counter; bumped on despawn.
    generations: Vec<u32>,
    /// Per-slot liveness flag (O(1) lookup for alive_mask construction).
    alive: Vec<bool>,
    /// FIFO queue of free slot indices ready for reuse.
    free_list: VecDeque<u32>,
    /// Number of currently alive entities.
    alive_count: u32,
    /// Slot indices whose alive state changed since the last GPU sync.
    pending_sync: Vec<u32>,
    /// Entities despawned since the last cleanup (for component removal).
    pending_despawn: Vec<Entity>,
}

impl EntityAllocator {
    /// Creates an allocator with the default capacity (1024 slots).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Creates an allocator pre-allocating `capacity` slots.
    ///
    /// All slots start as free. The free-list is filled in ascending order
    /// so the first spawns get indices 0, 1, 2, ...
    pub fn with_capacity(capacity: u32) -> Self {
        let cap = capacity as usize;
        let mut free_list = VecDeque::with_capacity(cap);
        for i in 0..capacity {
            free_list.push_back(i);
        }

        Self {
            generations: vec![0; cap],
            alive: vec![false; cap],
            free_list,
            alive_count: 0,
            pending_sync: Vec::new(),
            pending_despawn: Vec::new(),
        }
    }

    /// Spawns a new entity, returning its handle.
    ///
    /// Reuses a recycled slot when available; otherwise grows storage by
    /// doubling the current capacity.
    pub fn spawn(&mut self) -> Entity {
        let index = if let Some(idx) = self.free_list.pop_front() {
            idx
        } else {
            self.grow();
            self.free_list
                .pop_front()
                .expect("free_list should not be empty after grow")
        };

        self.alive[index as usize] = true;
        self.alive_count += 1;
        self.pending_sync.push(index);

        Entity::new(index, self.generations[index as usize])
    }

    /// Spawns `count` entities in one call.
    pub fn spawn_batch(&mut self, count: u32) -> Vec<Entity> {
        let mut batch = Vec::with_capacity(count as usize);
        for _ in 0..count {
            batch.push(self.spawn());
        }
        batch
    }

    /// Despawns an entity, incrementing its generation and returning the
    /// slot to the free-list.
    ///
    /// Returns `true` if the entity was alive and has been successfully
    /// despawned, `false` if the handle was stale or already dead.
    pub fn despawn(&mut self, entity: Entity) -> bool {
        let idx = entity.index() as usize;

        if idx >= self.generations.len() {
            return false;
        }

        // Stale handle or already dead.
        if self.generations[idx] != entity.generation() || !self.alive[idx] {
            return false;
        }

        self.alive[idx] = false;
        self.generations[idx] = self.generations[idx].wrapping_add(1);
        self.alive_count -= 1;
        self.free_list.push_back(entity.index());
        self.pending_sync.push(entity.index());
        self.pending_despawn.push(entity);

        true
    }

    /// Returns `true` if the entity handle still refers to a living entity.
    pub fn is_alive(&self, entity: Entity) -> bool {
        let idx = entity.index() as usize;
        idx < self.generations.len()
            && self.generations[idx] == entity.generation()
            && self.alive[idx]
    }

    /// Returns `true` if the slot at `index` is currently alive.
    ///
    /// This only checks liveness, not generation — useful when building
    /// the GPU alive_mask from raw indices.
    pub fn is_index_alive(&self, index: u32) -> bool {
        (index as usize) < self.alive.len() && self.alive[index as usize]
    }

    /// Number of entities currently alive.
    #[inline]
    pub fn alive_count(&self) -> u32 {
        self.alive_count
    }

    /// Total number of allocated slots (alive + free).
    #[inline]
    pub fn total_slots(&self) -> u32 {
        self.generations.len() as u32
    }

    /// Drains and returns slot indices that changed since the last call.
    ///
    /// The GPU sync system calls this once per frame to know which slots
    /// need their `alive_mask` value updated.
    pub fn take_pending_sync(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.pending_sync)
    }

    /// Drains and returns entities despawned since the last call.
    ///
    /// The component cleanup system calls this to remove despawned
    /// entities from all component storages.
    pub fn take_pending_despawn(&mut self) -> Vec<Entity> {
        std::mem::take(&mut self.pending_despawn)
    }

    /// Attempts to revive a previously despawned entity at its original slot.
    ///
    /// This is used by the undo system to restore an entity after a despawn
    /// has been undone, preserving the original `Entity` handle so that any
    /// references to it remain valid.
    ///
    /// Returns `true` if the entity was successfully revived. Returns `false`
    /// if the slot has been reused (generation advanced beyond +1) or the
    /// entity is still alive.
    pub fn revive(&mut self, entity: Entity) -> bool {
        let idx = entity.index() as usize;

        if idx >= self.generations.len() {
            return false;
        }

        // The entity must be dead and its generation must be exactly +1
        // from what the handle holds (meaning no other entity has reused
        // this slot since the despawn).
        if self.alive[idx] || self.generations[idx] != entity.generation().wrapping_add(1) {
            return false;
        }

        // Decrement generation back to the original value.
        self.generations[idx] = entity.generation();
        self.alive[idx] = true;
        self.alive_count += 1;
        self.pending_sync.push(entity.index());

        // Remove the slot from the free-list.
        self.free_list.retain(|&slot| slot != entity.index());

        true
    }

    // -- private --

    /// Doubles the capacity, pushing new indices onto the free-list.
    fn grow(&mut self) {
        let old_cap = self.generations.len() as u32;
        let new_cap = old_cap * 2;
        let added = new_cap - old_cap;

        self.generations.resize(new_cap as usize, 0);
        self.alive.resize(new_cap as usize, false);

        for i in old_cap..new_cap {
            self.free_list.push_back(i);
        }

        tracing::debug!(
            old = old_cap,
            new = new_cap,
            added,
            "EntityAllocator grew"
        );
    }
}

impl Default for EntityAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

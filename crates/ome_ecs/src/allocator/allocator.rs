//! [`EntityAllocator`] implementation.

use std::collections::VecDeque;

use crate::entity::Entity;

/// Default number of pre-allocated entity slots.
const DEFAULT_CAPACITY: u32 = 1024;

/// Allocates and recycles [`Entity`] handles with generational tracking.
///
/// Every spawn/despawn is recorded in a pending-sync list so the GPU
/// `alive_mask` buffer can be updated incrementally.
///
/// [`Clone`] so a world snapshot can put generations and the free list
/// back exactly as they were, rather than continuing past them — see
/// [`WorldSnapshot`](crate::world_snapshot::WorldSnapshot).
#[derive(Clone)]
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

    /// Marks every slot as needing a GPU alive-mask sync.
    ///
    /// Used after a wholesale world replacement, where the incremental
    /// dirty list no longer describes what changed.
    pub fn mark_all_pending_sync(&mut self) {
        self.pending_sync.clear();
        self.pending_sync.extend(0..self.generations.len() as u32);
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

        tracing::debug!(old = old_cap, new = new_cap, added, "EntityAllocator grew");
    }
}

impl Default for EntityAllocator {
    fn default() -> Self {
        Self::new()
    }
}

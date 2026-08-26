//! [`ChunkManager`] — central registry of in-memory chunks plus the
//! load/unload pipeline and eviction-hook plumbing.
//!
//! What this manager owns:
//! - `active`: HashMap of every chunk currently tracked.
//! - `load_queue`: priority queue (smallest distance² = highest priority).
//! - `unload_queue`: FIFO of chunks scheduled for eviction.
//! - `memory_budget_bytes`: cap; eviction is triggered when exceeded.
//! - `listeners`: callbacks invoked BEFORE a `Loaded` chunk transitions
//!   to `Unloading`. The hook is the integration point for #309 Edit
//!   Baker (flush deltas), #312 persistent edit log (commit on
//!   eviction), and similar lifecycle observers.
//!
//! The actual loading is **synchronous in this warmup**: `process_queues`
//! moves a chunk from queue → `active` with `state = Loaded` in one
//! step. When async loading lands (separate issue), the same API gains
//! `Loading{progress}` / `Unloading` intermediate states without
//! callsite changes.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::chunk::{ChunkData, ChunkId, ChunkState};

/// Listener invoked when a chunk is about to be evicted (transition
/// `Loaded → Unloading` in async, or `Loaded → removed` in the
/// synchronous warmup path).
///
/// Implementations: #309 Edit Baker flushes deltas to the sparse
/// baseline; #312 persistent edit log commits the on-disk file before
/// the chunk pages out. Order of invocation across multiple listeners
/// is registration order.
pub trait ChunkEvictionListener: Send + Sync {
    fn on_evict(&self, id: ChunkId);
}

/// Internal queue entry. `priority` is a squared distance (smaller =
/// closer = higher priority). Equality / ordering compare on
/// `priority` only; the heap doesn't need to break ties on `id`.
#[derive(Clone, Copy, Debug)]
struct LoadRequest {
    priority: f32,
    id: ChunkId,
}

impl PartialEq for LoadRequest {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl Eq for LoadRequest {}

impl PartialOrd for LoadRequest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LoadRequest {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap; we want smallest priority popped
        // first. Reverse the comparison.
        other
            .priority
            .partial_cmp(&self.priority)
            .unwrap_or(Ordering::Equal)
    }
}

/// Central chunk registry + load/unload pipeline.
///
/// Inserted as a `Resources` resource by [`super::plugin::WorldStreamingPlugin`].
pub struct ChunkManager {
    pub active: HashMap<ChunkId, ChunkData>,
    load_queue: BinaryHeap<LoadRequest>,
    unload_queue: Vec<ChunkId>,
    pub memory_budget_bytes: u64,
    /// Bytes currently held by loaded chunks. Maintained externally —
    /// callers tell the manager how much each load adds (heavy data
    /// lives in #136 sparse storage / #115 BVH and the warmup keeps
    /// this at zero).
    pub memory_used_bytes: u64,
    listeners: Vec<Box<dyn ChunkEvictionListener>>,
    /// Chunks that hit `process_queues`'s eviction path since the last
    /// drain. Renderer reads + clears this list to mirror the pool
    /// state. Decoupled from the load buffer so a chunk that loads and
    /// unloads in the same frame produces both a load and an unload
    /// event — the pool keeps its insert/remove invariants regardless
    /// of frame rate.
    pending_unloads: Vec<ChunkId>,
}

impl ChunkManager {
    pub fn new(memory_budget_bytes: u64) -> Self {
        Self {
            active: HashMap::new(),
            load_queue: BinaryHeap::new(),
            unload_queue: Vec::new(),
            memory_budget_bytes,
            memory_used_bytes: 0,
            listeners: Vec::new(),
            pending_unloads: Vec::new(),
        }
    }

    /// Register an eviction observer. Listeners fire in registration
    /// order on each evicted chunk.
    pub fn register_listener(&mut self, listener: Box<dyn ChunkEvictionListener>) {
        self.listeners.push(listener);
    }

    /// Drain chunks evicted by the most recent `process_queues` pass.
    /// Caller mirrors the eviction into the GPU pool via
    /// `KoochAccel::remove_chunk`.
    pub fn drain_pending_unloads(&mut self) -> Vec<ChunkId> {
        std::mem::take(&mut self.pending_unloads)
    }

    /// Request that the chunk be loaded. Idempotent: requesting an
    /// already-active chunk is a no-op. Duplicate queued requests
    /// dedup at process time.
    pub fn request_load(&mut self, id: ChunkId, priority: f32) {
        if self.active.contains_key(&id) {
            return;
        }
        self.load_queue.push(LoadRequest { priority, id });
    }

    /// Request that the chunk be unloaded. No-op if it's not currently
    /// active.
    pub fn request_unload(&mut self, id: ChunkId) {
        if !self.active.contains_key(&id) {
            return;
        }
        self.unload_queue.push(id);
    }

    /// Drain up to `max_loads` and `max_unloads` queued operations.
    /// Returns `(loaded_count, unloaded_count)` — useful for telemetry
    /// and for tuning the per-frame budget.
    ///
    /// Eviction listeners fire synchronously inside this call — they
    /// are observation hooks, not GPU coordinators.
    ///
    pub fn process_queues(&mut self, max_loads: usize, max_unloads: usize) -> (usize, usize) {
        let mut loaded = 0;
        let mut unloaded = 0;

        // Unloads first — frees up space for the loads queued behind
        // them.
        while unloaded < max_unloads {
            let Some(id) = self.unload_queue.pop() else {
                break;
            };
            if let Some(data) = self.active.remove(&id) {
                if matches!(data.state, ChunkState::Loaded) {
                    for l in &self.listeners {
                        l.on_evict(id);
                    }
                    self.pending_unloads.push(id);
                }
                unloaded += 1;
            }
        }

        // Then loads. Skip queue entries that became active in the
        // meantime (duplicate requests, requests for a chunk loaded
        // synchronously by something else).
        while loaded < max_loads {
            let Some(req) = self.load_queue.pop() else {
                break;
            };
            if self.active.contains_key(&req.id) {
                continue;
            }
            self.active.insert(
                req.id,
                ChunkData {
                    id: req.id,
                    state: ChunkState::Loaded,
                    last_seen_frame: 0,
                },
            );
            loaded += 1;
        }

        (loaded, unloaded)
    }

    /// Number of chunks currently in [`ChunkState::Loaded`].
    pub fn loaded_count(&self) -> usize {
        self.active
            .values()
            .filter(|d| matches!(d.state, ChunkState::Loaded))
            .count()
    }

    /// Number of pending load requests (after dedup against `active`).
    pub fn pending_load_count(&self) -> usize {
        self.load_queue.len()
    }

    /// Number of pending unload requests.
    pub fn pending_unload_count(&self) -> usize {
        self.unload_queue.len()
    }
}

impl Default for ChunkManager {
    fn default() -> Self {
        // 2 GB default — matches the issue body example.
        Self::new(2 * 1024 * 1024 * 1024)
    }
}

#[cfg(test)]
mod tests;

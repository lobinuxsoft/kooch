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
        }
    }

    /// Register an eviction observer. Listeners fire in registration
    /// order on each evicted chunk.
    pub fn register_listener(&mut self, listener: Box<dyn ChunkEvictionListener>) {
        self.listeners.push(listener);
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
mod tests {
    use super::*;
    use glam::IVec3;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    fn id(x: i32, y: i32, z: i32, level: u8) -> ChunkId {
        ChunkId::new(IVec3::new(x, y, z), level)
    }

    #[test]
    fn new_manager_is_empty() {
        let m = ChunkManager::new(1024);
        assert_eq!(m.loaded_count(), 0);
        assert_eq!(m.pending_load_count(), 0);
        assert_eq!(m.pending_unload_count(), 0);
        assert_eq!(m.memory_budget_bytes, 1024);
        assert_eq!(m.memory_used_bytes, 0);
    }

    #[test]
    fn request_load_then_process_loads_chunk() {
        let mut m = ChunkManager::new(1024);
        m.request_load(id(0, 0, 0, 0), 1.0);
        assert_eq!(m.pending_load_count(), 1);
        let (loaded, unloaded) = m.process_queues(10, 10);
        assert_eq!(loaded, 1);
        assert_eq!(unloaded, 0);
        assert_eq!(m.loaded_count(), 1);
    }

    #[test]
    fn request_load_dedupes_against_active() {
        let mut m = ChunkManager::new(1024);
        m.request_load(id(0, 0, 0, 0), 1.0);
        m.process_queues(10, 10);
        // Same chunk requested again — silently dropped.
        m.request_load(id(0, 0, 0, 0), 0.5);
        assert_eq!(m.pending_load_count(), 0);
    }

    #[test]
    fn closest_chunk_loaded_first() {
        let mut m = ChunkManager::new(1024);
        m.request_load(id(10, 0, 0, 0), 100.0);
        m.request_load(id(1, 0, 0, 0), 1.0);
        m.request_load(id(5, 0, 0, 0), 25.0);
        // Drain ONE — must be the lowest-priority (closest) chunk.
        m.process_queues(1, 0);
        assert!(m.active.contains_key(&id(1, 0, 0, 0)));
        assert!(!m.active.contains_key(&id(10, 0, 0, 0)));
        assert!(!m.active.contains_key(&id(5, 0, 0, 0)));
    }

    #[test]
    fn unload_invokes_listeners() {
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        struct Listener(std::sync::Arc<AtomicUsize>);
        impl ChunkEvictionListener for Listener {
            fn on_evict(&self, _id: ChunkId) {
                self.0.fetch_add(1, AtomicOrdering::Relaxed);
            }
        }

        let mut m = ChunkManager::new(1024);
        m.register_listener(Box::new(Listener(counter.clone())));
        m.request_load(id(0, 0, 0, 0), 1.0);
        m.process_queues(10, 10);
        m.request_unload(id(0, 0, 0, 0));
        m.process_queues(0, 10);

        assert_eq!(counter.load(AtomicOrdering::Relaxed), 1);
        assert_eq!(m.loaded_count(), 0);
    }

    #[test]
    fn unload_no_op_when_not_active() {
        let mut m = ChunkManager::new(1024);
        m.request_unload(id(99, 99, 99, 0));
        let (loaded, unloaded) = m.process_queues(0, 10);
        assert_eq!(loaded, 0);
        assert_eq!(unloaded, 0);
    }

    #[test]
    fn budget_per_frame_caps_processing() {
        let mut m = ChunkManager::new(1024);
        for i in 0..10 {
            m.request_load(id(i, 0, 0, 0), i as f32);
        }
        let (loaded, _) = m.process_queues(3, 0);
        assert_eq!(loaded, 3);
        assert_eq!(m.loaded_count(), 3);
        assert_eq!(m.pending_load_count(), 7);
    }

    #[test]
    fn default_budget_is_2gb() {
        let m = ChunkManager::default();
        assert_eq!(m.memory_budget_bytes, 2 * 1024 * 1024 * 1024);
    }
}

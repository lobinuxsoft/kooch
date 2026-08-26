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

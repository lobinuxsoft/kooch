use super::*;

#[test]
fn chunk_size_doubles_per_level() {
    let l0 = ChunkId::new(IVec3::ZERO, 0);
    let l1 = ChunkId::new(IVec3::ZERO, 1);
    let l3 = ChunkId::new(IVec3::ZERO, 3);
    assert!((l0.size_meters() - BASE_CHUNK_SIZE_METERS).abs() < 1e-6);
    assert!((l1.size_meters() - BASE_CHUNK_SIZE_METERS * 2.0).abs() < 1e-6);
    assert!((l3.size_meters() - BASE_CHUNK_SIZE_METERS * 8.0).abs() < 1e-6);
}

#[test]
fn id_equality_requires_both_fields() {
    let a = ChunkId::new(IVec3::new(1, 2, 3), 0);
    let b = ChunkId::new(IVec3::new(1, 2, 3), 0);
    let c = ChunkId::new(IVec3::new(1, 2, 3), 1);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn id_hashable_for_hashmap() {
    use std::collections::HashMap;
    let mut m: HashMap<ChunkId, u32> = HashMap::new();
    m.insert(ChunkId::new(IVec3::new(0, 0, 0), 0), 42);
    assert_eq!(m.get(&ChunkId::new(IVec3::new(0, 0, 0), 0)), Some(&42));
    // Same coords, different level → not the same chunk.
    assert_eq!(m.get(&ChunkId::new(IVec3::new(0, 0, 0), 1)), None);
}

#[test]
fn chunk_state_default_is_unloaded() {
    let data = ChunkData::new(ChunkId::new(IVec3::ZERO, 0));
    assert_eq!(data.state, ChunkState::Unloaded);
    assert_eq!(data.last_seen_frame, 0);
}

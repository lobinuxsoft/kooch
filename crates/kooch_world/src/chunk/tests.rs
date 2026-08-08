use super::*;

const EPS: f32 = 1e-3;

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
fn world_origin_uses_chunk_size() {
    let id = ChunkId::new(IVec3::new(2, -1, 0), 0);
    let world = id.world_origin().to_dvec3();
    assert!((world.x - 2.0 * BASE_CHUNK_SIZE_METERS).abs() < 1e-3);
    assert!((world.y - -1.0 * BASE_CHUNK_SIZE_METERS).abs() < 1e-3);
    assert!(world.z.abs() < 1e-3);
}

#[test]
fn bounds_at_zero_origin_match_world_origin() {
    let origin = ActiveOrigin::ZERO;
    let id = ChunkId::new(IVec3::ZERO, 0);
    let b = id.bounds(&origin);
    assert!((b.min - Vec3::ZERO).length() < EPS);
    assert!((b.max - Vec3::splat(BASE_CHUNK_SIZE_METERS as f32)).length() < EPS);
}

#[test]
fn bounds_shift_with_active_origin() {
    // Chunk at world (0,0,0); active origin shifted +100 m on x.
    // The chunk should appear at -100 m in the simulation frame.
    let origin = ActiveOrigin::new(UniverseCoord::from_dvec3(DVec3::new(100.0, 0.0, 0.0)));
    let id = ChunkId::new(IVec3::ZERO, 0);
    let b = id.bounds(&origin);
    assert!((b.min.x - (-100.0)).abs() < EPS);
    assert!((b.max.x - (-100.0 + BASE_CHUNK_SIZE_METERS as f32)).abs() < EPS);
}

#[test]
fn chunk_state_default_is_unloaded() {
    let data = ChunkData::new(ChunkId::new(IVec3::ZERO, 0));
    assert_eq!(data.state, ChunkState::Unloaded);
    assert_eq!(data.last_seen_frame, 0);
}

use super::*;

fn entity(idx: u32) -> Entity {
    Entity::new(idx, 0)
}

#[test]
fn first_call_reports_all_first_seen() {
    let mut cache = FocusCacheState::default();
    let focuses = vec![(entity(1), DVec3::ZERO)];
    let dirty = cache.dirty_pairs(&focuses, 4);
    assert_eq!(dirty.len(), 4); // one per LOD ring
    assert!(dirty.iter().all(|d| d.is_first_seen()));
    assert_eq!(cache.tracked_count(), 1);
}

#[test]
fn stationary_focus_no_dirty() {
    let mut cache = FocusCacheState::default();
    let focuses = vec![(entity(1), DVec3::new(5.0, 5.0, 5.0))];
    // First call seeds the cache.
    let _ = cache.dirty_pairs(&focuses, 4);
    // Second call with same position: nothing dirty.
    let dirty = cache.dirty_pairs(&focuses, 4);
    assert_eq!(dirty.len(), 0);
}

#[test]
fn movement_within_same_chunk_no_dirty() {
    let mut cache = FocusCacheState::default();
    // Chunk size at LOD 0 is 64 m; two points 10 m apart at the
    // same chunk index.
    let _ = cache.dirty_pairs(&[(entity(1), DVec3::new(10.0, 10.0, 10.0))], 4);
    let dirty = cache.dirty_pairs(&[(entity(1), DVec3::new(20.0, 30.0, 40.0))], 4);
    assert_eq!(dirty.len(), 0, "still chunk (0,0,0) at all LODs");
}

#[test]
fn cross_chunk_boundary_marks_dirty_at_lod_0() {
    let mut cache = FocusCacheState::default();
    let _ = cache.dirty_pairs(&[(entity(1), DVec3::new(10.0, 10.0, 10.0))], 4);
    // Move past the LOD-0 boundary on x (64 m). Higher LODs (128,
    // 256, 512 m) still resolve to the same cell.
    let dirty = cache.dirty_pairs(&[(entity(1), DVec3::new(70.0, 10.0, 10.0))], 4);
    assert_eq!(dirty.len(), 1);
    assert_eq!(dirty[0].lod, 0);
    assert_eq!(dirty[0].current, IVec3::new(1, 0, 0));
    assert_eq!(dirty[0].previous, IVec3::ZERO);
    assert!(!dirty[0].is_first_seen());
}

#[test]
fn long_jump_marks_all_lods_dirty() {
    let mut cache = FocusCacheState::default();
    let _ = cache.dirty_pairs(&[(entity(1), DVec3::ZERO)], 4);
    // Jump 10 km on x: crosses every LOD's boundary (64, 128, 256,
    // 512 m).
    let dirty = cache.dirty_pairs(&[(entity(1), DVec3::new(10_000.0, 0.0, 0.0))], 4);
    assert_eq!(dirty.len(), 4);
}

#[test]
fn only_moved_focus_is_dirty() {
    let mut cache = FocusCacheState::default();
    let initial = vec![
        (entity(1), DVec3::new(10.0, 10.0, 10.0)),
        (entity(2), DVec3::new(500.0, 0.0, 0.0)),
    ];
    let _ = cache.dirty_pairs(&initial, 4);
    // Only entity 1 crosses a boundary; entity 2 stays put.
    let next = vec![
        (entity(1), DVec3::new(70.0, 10.0, 10.0)),
        (entity(2), DVec3::new(500.0, 0.0, 0.0)),
    ];
    let dirty = cache.dirty_pairs(&next, 4);
    assert_eq!(dirty.len(), 1);
    assert_eq!(dirty[0].entity, entity(1));
}

#[test]
fn purge_stale_drops_missing_entities() {
    let mut cache = FocusCacheState::default();
    let _ = cache.dirty_pairs(
        &[
            (entity(1), DVec3::ZERO),
            (entity(2), DVec3::new(100.0, 0.0, 0.0)),
            (entity(3), DVec3::new(-100.0, 0.0, 0.0)),
        ],
        4,
    );
    assert_eq!(cache.tracked_count(), 3);
    // Only entity(2) survives.
    cache.purge_stale(&[(entity(2), DVec3::new(100.0, 0.0, 0.0))]);
    assert_eq!(cache.tracked_count(), 1);
}

#[test]
fn lod_count_change_resizes_cache() {
    let mut cache = FocusCacheState::default();
    let _ = cache.dirty_pairs(&[(entity(1), DVec3::ZERO)], 4);
    // Simulate a LodRingConfig change at runtime — fewer rings.
    let dirty = cache.dirty_pairs(&[(entity(1), DVec3::ZERO)], 2);
    // Cache is resized; no false positives.
    assert_eq!(dirty.len(), 0);
}

#[test]
fn negative_position_floors_correctly() {
    let mut cache = FocusCacheState::default();
    // -10 / 64 = -0.156 → floor = -1. Avoid the rust-default
    // truncate-toward-zero pitfall.
    let dirty = cache.dirty_pairs(&[(entity(1), DVec3::new(-10.0, 0.0, 0.0))], 1);
    assert_eq!(dirty[0].current.x, -1);
}

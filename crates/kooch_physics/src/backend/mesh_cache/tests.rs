use super::*;

fn triangle() -> ColliderMesh {
    ColliderMesh {
        vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
        indices: vec![[0, 1, 2]],
    }
}

#[test]
fn an_unanswered_guid_reads_zero() {
    let cache = ColliderMeshCache::new();
    assert_eq!(cache.epoch(Guid::new_v4()), 0);
    assert!(!cache.answered(Guid::new_v4()));
}

/// What makes a mesh arriving *after* the body was authored rebuild it:
/// the spec carries this number, and the body is retired when it moves.
#[test]
fn arriving_moves_the_epoch() {
    let mut cache = ColliderMeshCache::new();
    let guid = Guid::new_v4();
    cache.insert(guid, triangle());
    assert!(cache.epoch(guid) > 0);
    assert_eq!(cache.get(guid), Some(&triangle()));
}

/// The filler runs every frame. A failure that has not changed must not
/// bump the epoch, or every body naming a broken mesh rebuilds forever.
#[test]
fn repeating_a_failure_is_quiet() {
    let mut cache = ColliderMeshCache::new();
    let guid = Guid::new_v4();
    cache.fail(guid);
    let first = cache.epoch(guid);
    cache.fail(guid);
    assert_eq!(cache.epoch(guid), first);
    assert!(cache.answered(guid), "a failure is still an answer");
    assert_eq!(cache.get(guid), None);
}

/// A GUID cleared and refilled with the same mesh still has to read as a
/// change, or a body built from the old data never rebuilds.
#[test]
fn clearing_keeps_counting() {
    let mut cache = ColliderMeshCache::new();
    let guid = Guid::new_v4();
    cache.insert(guid, triangle());
    let before = cache.epoch(guid);
    cache.clear();
    cache.insert(guid, triangle());
    assert!(cache.epoch(guid) > before);
}

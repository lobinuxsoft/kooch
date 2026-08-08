use super::*;

/// Saving one prefab twice before the drain runs must not propagate it
/// twice, and draining has to leave the queue empty or the same work
/// repeats every frame.
#[test]
fn the_queue_holds_each_prefab_once() {
    let mut pending = PendingPropagation::default();
    let prefab = Guid::new_v4();
    pending.queue(prefab);
    pending.queue(prefab);
    assert_eq!(pending.drain(), vec![prefab]);
    assert!(pending.drain().is_empty());
}

/// The idle-frame guard asks this. If it ever answered wrongly,
/// propagation would go back to only happening when the user happened
/// to do something else.
#[test]
fn a_queued_prefab_is_visible_to_the_frame_guard() {
    let mut resources = Resources::new();
    assert!(!anything_queued(&resources));
    queue(&mut resources, Guid::new_v4());
    assert!(anything_queued(&resources));
}

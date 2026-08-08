use super::*;

/// Draining has to empty the queue, or a listener reading every frame
/// sees the same collision forever.
#[test]
fn draining_empties_the_queue() {
    let collector = EventCollector::default();
    let mut colliders = ColliderSet::new();
    let handle = colliders.insert(rapier3d::prelude::ColliderBuilder::ball(0.5).build());

    collector.handle_collision_event(
        &RigidBodySet::new(),
        &colliders,
        RapierCollisionEvent::Started(handle, handle, CollisionEventFlags::empty()),
        None,
    );

    assert_eq!(collector.drain_collisions().len(), 1);
    assert!(
        collector.drain_collisions().is_empty(),
        "the same event drained twice",
    );
}

/// No contact pair is how a sensor overlap arrives, and losing that
/// distinction would have listeners looking for a manifold that was
/// never computed.
#[test]
fn a_missing_contact_pair_marks_a_sensor_overlap() {
    let collector = EventCollector::default();
    let mut colliders = ColliderSet::new();
    let handle = colliders.insert(rapier3d::prelude::ColliderBuilder::ball(0.5).build());

    collector.handle_collision_event(
        &RigidBodySet::new(),
        &colliders,
        RapierCollisionEvent::Started(handle, handle, CollisionEventFlags::empty()),
        None,
    );

    let drained = collector.drain_collisions();
    assert!(drained[0].sensor);
    assert!(drained[0].started);
}

#[test]
fn a_stopped_event_is_not_a_start() {
    let collector = EventCollector::default();
    let mut colliders = ColliderSet::new();
    let handle = colliders.insert(rapier3d::prelude::ColliderBuilder::ball(0.5).build());

    collector.handle_collision_event(
        &RigidBodySet::new(),
        &colliders,
        RapierCollisionEvent::Stopped(handle, handle, CollisionEventFlags::empty()),
        None,
    );

    assert!(!collector.drain_collisions()[0].started);
}

/// An empty manifold set must give zero rather than panicking on an
/// empty fold — which is what a sensor pair looks like.
#[test]
fn a_pair_with_no_manifolds_peaks_at_zero() {
    assert_eq!(peak_force(&ContactPair::default()), 0.0);
}

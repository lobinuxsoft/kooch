//! Collecting rapier's events during a step, for delivery after it.
//!
//! # Why a mutex over a plain `Vec`
//!
//! Rapier's [`EventHandler`] takes `&self`, and requires `Send + Sync`: it
//! is called from inside the step, potentially from the solver's worker
//! threads. So a collector needs interior mutability that is also `Sync`.
//!
//! Rapier's own `ChannelEventCollector` solves this with crossbeam
//! channels. A [`Mutex<Vec<_>>`] does the same job without another
//! dependency, and the contention is a handful of pushes per step against a
//! lock nothing else wants.
//!
//! [`EventHandler`]: rapier3d::prelude::EventHandler

use std::sync::Mutex;

use rapier3d::prelude::{
    ColliderHandle as RapierColliderHandle, ColliderSet, CollisionEvent as RapierCollisionEvent,
    CollisionEventFlags, ContactPair, EventHandler, RigidBodyHandle, RigidBodySet,
};

/// What one step reported, in rapier's own terms.
///
/// Kept in rapier handles here and translated to [`BodyHandle`] on drain:
/// the translation needs the backend's mapping, and the handler runs where
/// the backend is already borrowed by the step.
///
/// [`BodyHandle`]: crate::backend::BodyHandle
#[derive(Default)]
pub(super) struct EventCollector {
    collisions: Mutex<Vec<RawCollision>>,
    forces: Mutex<Vec<RawForce>>,
}

pub(super) struct RawCollision {
    pub colliders: (RapierColliderHandle, RapierColliderHandle),
    pub started: bool,
    /// Rapier tells us by handing over no contact pair — a sensor has no
    /// manifold to hand over.
    pub sensor: bool,
}

pub(super) struct RawForce {
    pub colliders: (RapierColliderHandle, RapierColliderHandle),
    pub total_force_magnitude: f32,
    pub max_force_magnitude: f32,
}

impl EventCollector {
    /// Takes everything collected since the last drain.
    ///
    /// A poisoned lock is treated as empty rather than propagated: a panic
    /// in a solver worker is already being reported, and turning it into a
    /// second panic inside the drain buries the first.
    pub(super) fn drain_collisions(&self) -> Vec<RawCollision> {
        self.collisions
            .lock()
            .map(|mut queue| std::mem::take(&mut *queue))
            .unwrap_or_default()
    }

    pub(super) fn drain_forces(&self) -> Vec<RawForce> {
        self.forces
            .lock()
            .map(|mut queue| std::mem::take(&mut *queue))
            .unwrap_or_default()
    }
}

impl EventHandler for EventCollector {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: RapierCollisionEvent,
        contact_pair: Option<&ContactPair>,
    ) {
        let (started, colliders) = match event {
            RapierCollisionEvent::Started(a, b, _) => (true, (a, b)),
            RapierCollisionEvent::Stopped(a, b, _) => (false, (a, b)),
        };
        if let Ok(mut queue) = self.collisions.lock() {
            queue.push(RawCollision {
                colliders,
                started,
                // No contact pair means no manifold, which is what a
                // sensor overlap looks like from here.
                sensor: contact_pair.is_none(),
            });
        }
    }

    fn handle_contact_force_event(
        &self,
        _dt: f32,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        contact_pair: &ContactPair,
        total_force_magnitude: f32,
    ) {
        if let Ok(mut queue) = self.forces.lock() {
            queue.push(RawForce {
                colliders: (contact_pair.collider1, contact_pair.collider2),
                total_force_magnitude,
                // Rapier hands over the total; the peak is in the
                // manifolds, and telling a spread blow from a spike is the
                // reason to carry both.
                max_force_magnitude: peak_force(contact_pair),
            });
        }
    }
}

/// The largest single contact impulse in a pair.
fn peak_force(pair: &ContactPair) -> f32 {
    pair.manifolds
        .iter()
        .flat_map(|manifold| manifold.points.iter())
        .map(|point| point.data.impulse)
        .fold(0.0f32, f32::max)
}

/// Which rigid body owns a collider, if any.
///
/// A sensor with no parent body is legal — a static trigger volume authored
/// without a `RigidBody` — and produces `None` rather than an error.
pub(super) fn parent_of(
    colliders: &ColliderSet,
    collider: RapierColliderHandle,
) -> Option<RigidBodyHandle> {
    colliders.get(collider)?.parent()
}

#[cfg(test)]
mod tests {
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
}

//! Collects rapier events during a step. [`EventHandler`](rapier3d::prelude::EventHandler) takes
//! `&self`, `Send + Sync`, from worker threads; a [`Mutex<Vec<_>>`] does crossbeam's job without
//! the dependency.

use std::sync::Mutex;

use rapier3d::prelude::{
    ColliderHandle as RapierColliderHandle, ColliderSet, CollisionEvent as RapierCollisionEvent,
    ContactPair, EventHandler, RigidBodyHandle, RigidBodySet,
};

/// One step's reports in rapier handles, translated to [`BodyHandle`](crate::backend::BodyHandle)
/// on drain, where the mapping is borrowable.
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
    /// Takes everything collected; a poisoned lock reads empty rather than burying the worker's
    /// panic under a second.
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

/// The rigid body owning a collider; `None` for a parentless static trigger.
pub(super) fn parent_of(
    colliders: &ColliderSet,
    collider: RapierColliderHandle,
) -> Option<RigidBodyHandle> {
    colliders.get(collider)?.parent()
}

#[cfg(test)]
mod tests;

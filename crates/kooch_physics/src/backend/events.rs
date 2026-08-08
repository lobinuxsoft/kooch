//! What the solver reports back.
//!
//! # Collected during the step, delivered after it
//!
//! Rapier calls its event handler from inside `step`, while it holds the
//! whole world mutably. Gameplay cannot run there — a system that wanted to
//! despawn the thing it just collided with would be asking to mutate the
//! set being iterated. So the handler only *collects*, and the events are
//! drained afterwards.
//!
//! That ordering is not an implementation detail to tidy away later. It is
//! the difference between "a pickup disappears when touched" working and
//! deadlocking.
//!
//! # Bodies, not colliders
//!
//! Rapier reports collider pairs. These carry [`BodyHandle`], because a
//! compound body's third shape touching a wall is *the body* touching the
//! wall as far as gameplay is concerned, and a consumer that had to
//! resolve shapes back to owners would do it in every listener.
//!
//! The plugin layer turns these into `Entity`-carrying engine events, so
//! nothing above the seam ever sees a handle of either kind.

use super::body::BodyHandle;

/// Two bodies started or stopped touching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionEvent {
    pub a: BodyHandle,
    pub b: BodyHandle,
    /// `true` for the frame they began touching, `false` for the frame
    /// they stopped.
    pub started: bool,
    /// Whether this came from a sensor overlap rather than a solid
    /// contact.
    ///
    /// Worth carrying: a sensor event has no contact information behind it
    /// — rapier computes no manifold for a sensor — so a listener that
    /// wanted a contact point needs to know not to ask.
    pub sensor: bool,
}

/// Two bodies hit each other harder than one of them cared to ignore.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactForceEvent {
    pub a: BodyHandle,
    pub b: BodyHandle,
    /// Sum of the forces over the contact, in newtons.
    pub total_force_magnitude: f32,
    /// The largest single contact's force. A glancing blow spread over
    /// many points and a spike through one point can share a total; this
    /// is how a listener tells them apart.
    pub max_force_magnitude: f32,
}

#[cfg(test)]
mod tests;

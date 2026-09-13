//! Solver reports, collected in `step` and drained after, since rapier holds the world mutably
//! there. They carry [`BodyHandle`], not colliders: a compound body's shape touching is the body
//! touching.

use super::body::BodyHandle;

/// Two bodies started or stopped touching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionEvent {
    pub a: BodyHandle,
    pub b: BodyHandle,
    /// `true` for the frame they began touching, `false` for the frame
    /// they stopped.
    pub started: bool,
    /// A sensor overlap rather than a solid contact — rapier computes no manifold for sensors, so
    /// there is no contact point to ask for.
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

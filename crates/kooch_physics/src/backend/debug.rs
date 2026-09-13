//! The solver's own state as segments — contacts, centres of mass, anchors, bounds, sleep. The
//! collider gizmo draws components; a disagreement is the sync bug. Geometry only, rapier types
//! stay out of [`PhysicsBackend`](super::PhysicsBackend).

use glam::Vec3;

/// One world-space overlay segment, linear RGB, ready for a line renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugLine {
    pub start: Vec3,
    pub end: Vec3,
    pub color: Vec3,
}

/// Which parts of the solver to describe — separate switches, since contacts are cheap and shapes
/// expensive. All off by default: the walk costs CPU per shape per frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DebugCategories {
    /// Collider outlines as the solver holds them; off by default, as the expensive category the
    /// component gizmo mostly covers.
    pub collider_shapes: bool,
    /// Where bodies are actually touching. The question "is friction doing
    /// this" is unanswerable without it.
    pub contacts: bool,
    /// Joint anchors and the separation between them — a joint anchored to
    /// the wrong point looks identical to one that is not working.
    pub joints: bool,
    /// Broad-phase bounds. Mostly useful when something is not colliding
    /// at all and the question is whether the broad phase can even see it.
    pub collider_aabbs: bool,
    /// Each body's local axes, drawn at its **centre of mass** — the thing
    /// that made #618 impossible to diagnose by looking.
    pub body_axes: bool,
}

impl DebugCategories {
    /// Everything on. For a screenshot, or when you have no idea yet.
    pub fn all() -> Self {
        Self {
            collider_shapes: true,
            contacts: true,
            joints: true,
            collider_aabbs: true,
            body_axes: true,
        }
    }

    /// Whether anything is on; checked before asking, so a disabled overlay never walks.
    pub fn any(&self) -> bool {
        self.collider_shapes
            || self.contacts
            || self.joints
            || self.collider_aabbs
            || self.body_axes
    }
}

#[cfg(test)]
mod tests;

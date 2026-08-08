//! Which scene authored an entity.
//!
//! With more than one scene open, "who owns this entity" stops being
//! implicit. Saving has to write only its own entities, unloading has to
//! despawn only its own, and the World panel has to say which file a row
//! came from.
//!
//! # Why this is not serialised
//!
//! Every entity in a file belongs to that file's scene, so writing the
//! membership into the file stores the same fact twice and lets the two
//! copies disagree — a file claiming an entity belongs to a scene it is not
//! in is a contradiction nothing could resolve. It is assigned on load
//! instead, the same way [`Children`](crate::hierarchy::Children) and
//! [`GlobalTransform`](crate::hierarchy::GlobalTransform) are derived
//! rather than stored.
//!
//! # Scene membership is not cell residency
//!
//! This is the *authoring* home — a human decision, stored. Which cell an
//! entity is in is derived from its transform and changes as it moves
//! (#566). They are orthogonal: a scene spans many cells, and several
//! scenes can overlap one cell. Storing residency here would go stale the
//! first time something moved.

use kooch_core::Guid;

use crate::component::Component;

/// Marks the scene an entity was authored in.
///
/// Absent on entities that belong to no scene — editor cameras, gizmo
/// helpers, and anything else marked
/// [`ephemeral`](crate::ephemeral::EphemeralComponents).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneMember {
    pub scene: Guid,
}

impl SceneMember {
    pub const fn new(scene: Guid) -> Self {
        Self { scene }
    }
}

impl Component for SceneMember {}

#[cfg(test)]
mod tests;

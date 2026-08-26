//! What Ctrl+C is holding.
//!
//! # Why values and not entities
//!
//! The clipboard keeps [`EntityState`]s — names, component types, field
//! values — rather than the entities they were read from. A list of
//! handles would be a list of things that can be deleted between the copy
//! and the paste, and in remote mode it would be a list of *mirror*
//! handles, which mean nothing after a reconnect.
//!
//! So copying reads the world once, and pasting never looks at the source
//! again. Copy an entity, delete it, paste it: that works, and it is the
//! shape of the feature people expect from every other editor.
//!
//! # Not the system clipboard
//!
//! Deliberately in-process. The system clipboard carries text, and an
//! entity is not text — serialising one into it would invent a second
//! scene format alongside `.scene` and `.prefab`, and pasting into a text
//! editor would produce something that looks pasteable back and is not.
//! Moving entities between two editor windows is a real request and it
//! belongs to prefabs, which are the format for that (#611).

use crate::actions::entity_state::EntityState;

/// The editor's own clipboard, holding whatever was copied last.
///
/// Empty until the first copy, and never cleared afterwards: a clipboard
/// that empties itself is a clipboard that loses what you put in it while
/// you were doing something else.
#[derive(Default)]
pub(crate) struct EntityClipboard {
    states: Vec<EntityState>,
}

impl EntityClipboard {
    /// Replaces the contents. One copy, one clipboard — appending would
    /// make the second Ctrl+C paste twice as much as it copied.
    pub fn set(&mut self, states: Vec<EntityState>) {
        self.states = states;
    }

    pub fn states(&self) -> &[EntityState] {
        &self.states
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
}

//! What Ctrl+C is holding.

use crate::actions::entity_state::EntityState;

/// The editor's own clipboard, holding whatever was copied last.
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

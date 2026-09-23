//! What Ctrl+C is holding.

use crate::actions::entity_state::CapturedTree;

/// The editor's own clipboard, holding whatever was copied last — each entry a whole subtree,
/// because an entity with children is one thing to an author (#1292).
#[derive(Default)]
pub(crate) struct EntityClipboard {
    states: Vec<CapturedTree>,
}

impl EntityClipboard {
    /// Replaces the contents. One copy, one clipboard — appending would
    /// make the second Ctrl+C paste twice as much as it copied.
    pub fn set(&mut self, states: Vec<CapturedTree>) {
        self.states = states;
    }

    pub fn states(&self) -> &[CapturedTree] {
        &self.states
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
}

//! Name component — human-readable entity label.
//!
//! Built-in component used by the editor to display entity names
//! in the hierarchy and inspector panels.

use crate::component::Component;

// Import the derive macro (re-exported at crate root).
#[allow(unused_imports)]
use crate::Reflect;

/// Human-readable name for an entity.
///
/// The editor displays this in the World panel instead of the raw
/// `index:generation` when present and non-empty.
#[derive(Debug, Clone, Default, Reflect)]
#[reflect(inspector = "hidden")]
pub struct Name {
    pub value: String,
}

impl Component for Name {}

impl Name {
    /// Creates a new `Name` with the given value.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

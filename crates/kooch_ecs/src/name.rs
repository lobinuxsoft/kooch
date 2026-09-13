//! Name component — human-readable entity label.

use crate::component::Component;

// Import the derive macro (re-exported at crate root).
#[allow(unused_imports)]
use crate::Reflect;

/// Human-readable name for an entity.
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

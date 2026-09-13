//! Which systems are switched off right now.

use std::collections::HashSet;

use super::identity::SystemKey;

/// The systems that are not to run.
#[derive(Debug, Default, Clone)]
pub struct SystemToggles {
    off: HashSet<SystemKey>,
}

impl SystemToggles {
    /// Nothing switched off.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stops a system running, from the next frame.
    ///
    /// The name is canonicalised, so the path of a bare function reaches
    /// the system even when the schedule holds it wrapped:
    ///
    /// ```ignore
    /// toggles.disable(std::any::type_name_of_val(&spin_pivots));
    /// ```
    pub fn disable(&mut self, key: impl Into<SystemKey>) {
        self.off.insert(key.into());
    }

    /// Lets a system run again.
    pub fn enable(&mut self, key: impl Into<SystemKey>) {
        self.off.remove(&key.into());
    }

    /// Whether this system is switched off.
    pub fn is_disabled(&self, key: &SystemKey) -> bool {
        self.off.contains(key)
    }

    /// Every system currently switched off.
    pub fn disabled(&self) -> impl Iterator<Item = &SystemKey> {
        self.off.iter()
    }

    /// `true` when nothing is switched off.
    pub fn is_empty(&self) -> bool {
        self.off.is_empty()
    }

    /// How many systems are switched off.
    pub fn len(&self) -> usize {
        self.off.len()
    }
}

#[cfg(test)]
mod tests;

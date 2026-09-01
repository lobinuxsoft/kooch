//! Which systems are switched off right now.
//!
//! # Not running is not the same as not compiled in
//!
//! Leaving a system out of the binary is a cargo feature's job, and the
//! engine already does it that way — `kooch_ecs::testing` and
//! `physics-debug-render` both drop their components and systems
//! entirely when the feature is off. A runtime switch cannot do that and
//! must not pretend to.
//!
//! This is the other want: a system that IS compiled in and does not
//! need to run this frame, stopped without a rebuild — to isolate a cost,
//! or because a mode does not need it.
//!
//! # Skipped, never removed
//!
//! The schedule keeps every system it was given. Taking one out of the
//! `Vec` would mean parking a non-clonable, stateful `Box<dyn System>`
//! somewhere and remembering an index to put it back — positional
//! coupling in the one place worth avoiding it. And once GPU systems
//! exist, removal would silently re-batch the encoder around the gap.

use std::collections::HashSet;

use super::identity::SystemKey;

/// The systems that are not to run.
///
/// Absent from `Resources`, or absent from this set, means ON — so a
/// build nobody expressed an opinion about schedules exactly what it
/// scheduled before this existed.
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

    /// Switches everything back on.
    pub fn enable_all(&mut self) {
        self.off.clear();
    }

    /// Every system currently switched off.
    pub fn disabled(&self) -> impl Iterator<Item = &SystemKey> {
        self.off.iter()
    }

    /// `true` when nothing is switched off.
    ///
    /// 🔴 What `run_stage` reads first. The common case is that nobody
    /// touched anything, and this is what keeps that case free of a
    /// per-system hash lookup.
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

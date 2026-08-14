//! When two edits are one step.
//!
//! # The bug this is for
//!
//! The Inspector emits an edit on every `response.changed()`, so typing
//! `Player` into a name field is six edits and dragging a light's
//! intensity is one per frame. Every one of them was its own history
//! entry: undoing a rename took six Ctrl+Z, and undoing a drag took as
//! many as the drag had frames. The undo worked perfectly and looked
//! completely broken, which is the failure mode that made this the first
//! thing to fix.
//!
//! # The rule
//!
//! An edit merges into the one before it when they name the **same
//! target** and nothing has closed the group since. That is Godot's
//! `MERGE_ENDS` and Unity's collapsed undo group, arrived at the same
//! way: a continuous edit is a continuous edit, and the history should
//! hold what the user did rather than how many frames it took.
//!
//! What closes a group — the *seal* — is a mouse button coming up, a
//! text field losing focus, or the selection changing. Without it, a
//! field edited now would merge with the same field edited an hour ago,
//! and the step would claim to undo both.

use std::hash::{Hash, Hasher};

/// Identifies the target of an edit, so a run of them can be recognised.
///
/// A hash rather than the parts: the parts are a guid, an index, two
/// type names and a field name, in four different shapes depending on
/// what is being edited. Keeping them would mean a key type per
/// document kind, and all anyone asks of it is whether two edits point
/// at the same thing.
///
/// A collision merges two steps that should have stayed apart. At 64
/// bits, over the hundred steps a history holds, that is not a risk
/// worth a more complicated type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MergeKey(u64);

impl MergeKey {
    pub fn of(parts: impl Hash) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        parts.hash(&mut hasher);
        Self(hasher.finish())
    }
}

/// Whether a new edit continues the previous one.
///
/// Both halves are required: the same target, and a group that was never
/// closed. An edit with no key never merges — it is a discrete thing
/// (a spawn, a component added) and two of them are two steps.
pub(crate) fn continues(previous: Option<MergeKey>, next: Option<MergeKey>, sealed: bool) -> bool {
    if sealed {
        return false;
    }
    match (previous, next) {
        (Some(previous), Some(next)) => previous == next,
        _ => false,
    }
}

#[cfg(test)]
mod tests;

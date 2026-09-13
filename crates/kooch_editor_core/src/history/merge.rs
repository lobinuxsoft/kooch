//! When two edits are one step.

use std::hash::{Hash, Hasher};

/// Identifies the target of an edit, so a run of them can be recognised.
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

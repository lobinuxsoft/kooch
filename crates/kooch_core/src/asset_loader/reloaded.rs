//! How many times each asset has been rewritten.

use std::collections::HashMap;

use crate::Guid;

/// A counter per asset, bumped every time its file is written.
///
/// # Why a revision and not a queue
///
/// A reload **overwrites the value under the existing handle**, on
/// purpose: the world holds handles into `Assets<T>`, and replacing the
/// slot would leave every one of them pointing at the bytes from before
/// the edit. So nothing about the handle changes, and a consumer that
/// caches a derived thing — a block's generated mesh, its collider —
/// has no way to notice.
///
/// A queue would work for one consumer and break for two: whoever
/// drained it first would hide the change from the rest. A number each
/// of them compares against its own is read as many times as needed.
#[derive(Debug, Default)]
pub struct ReloadedAssets {
    revisions: HashMap<Guid, u64>,
}

impl ReloadedAssets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that `guid`'s file was written.
    pub fn bump(&mut self, guid: Guid) {
        *self.revisions.entry(guid).or_default() += 1;
    }

    /// How many times `guid` has been written. Zero for one that never
    /// has, so a consumer's first build compares equal to nothing and
    /// happens once.
    pub fn revision(&self, guid: Guid) -> u64 {
        self.revisions.get(&guid).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;

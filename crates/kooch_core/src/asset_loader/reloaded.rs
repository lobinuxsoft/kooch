//! How many times each asset has been rewritten.

use std::collections::HashMap;

use crate::Guid;

/// A counter per asset, bumped every time its file is written.
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

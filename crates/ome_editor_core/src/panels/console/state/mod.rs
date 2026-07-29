//! What the Console holds between frames, and why it holds anything.
//!
//! # The panel keeps its own copy of the log
//!
//! It used to call `LogBuffer::snapshot()` inside the draw, which clones
//! every line — two thousand entries, two `String`s each — **every frame**,
//! then filtered them twice (once to count, once to draw), lowercasing
//! three strings per entry per pass. A panel nobody was looking at cost
//! nothing; the one with the log open cost tens of thousands of
//! allocations a frame, and the cost grew as the log filled.
//!
//! [`LogEntry::seq`] existed for exactly this and was going unused. The
//! panel now holds its own `Vec` and asks only for what arrived since the
//! last line it has.
//!
//! # And its own filtered view
//!
//! Recomputed when the log or the filter settings change, not per frame.
//! The list is `usize` indices rather than cloned entries: the copy above
//! is the only one.

use ome_core::{LogBuffer, LogEntry};

mod levels;

pub(crate) use levels::{ALL as ALL_LEVELS, LevelSet};

/// What the panel is currently showing, kept between frames.
pub(crate) struct ConsoleState {
    /// Which severities are shown. A set rather than a threshold, so one
    /// noisy level can be hidden without taking the rest with it.
    pub(crate) levels: LevelSet,
    /// Substring filter, matched against the message and the target.
    pub(crate) filter: String,
    /// Follow the tail. Turned off by scrolling up, so reading something
    /// is not fought by new lines arriving.
    pub(crate) follow: bool,
    /// Show only what a spawned project said.
    pub(crate) project_only: bool,

    /// The panel's copy of the log, oldest first.
    entries: Vec<LogEntry>,
    /// Indices into [`entries`](Self::entries) that pass the filters.
    visible: Vec<usize>,
    /// The filter settings `visible` was built from, so a change to any of
    /// them is noticed without recomputing to find out.
    built_for: Option<FilterKey>,
    /// How many lines the log had dropped when last read.
    dropped: u64,
}

/// The filter settings, as something comparable.
#[derive(Clone, PartialEq, Eq)]
struct FilterKey {
    levels: LevelSet,
    filter: String,
    project_only: bool,
    /// The newest line included, so arriving lines invalidate the view
    /// even when the settings did not move.
    newest: Option<u64>,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self {
            levels: LevelSet::DEFAULT,
            filter: String::new(),
            follow: true,
            project_only: false,
            entries: Vec::new(),
            visible: Vec::new(),
            built_for: None,
            dropped: 0,
        }
    }
}

impl ConsoleState {
    /// Whether an entry passes the current filters.
    ///
    /// `needle` is the filter already lowercased, because lowercasing it
    /// per entry was most of what this function used to do.
    pub(crate) fn shows_with(&self, entry: &LogEntry, needle: &str) -> bool {
        if !self.levels.shows(entry.level) {
            return false;
        }
        if self.project_only && !entry.is_from_project() {
            return false;
        }
        if needle.is_empty() {
            return true;
        }
        contains_ignore_case(&entry.message, needle) || contains_ignore_case(&entry.target, needle)
    }

    /// Whether an entry passes, lowercasing the filter as it goes.
    ///
    /// For callers with one entry to check. The drawing path uses
    /// [`shows_with`](Self::shows_with) so the filter is folded once for
    /// the whole pass instead of once per line.
    pub(crate) fn shows(&self, entry: &LogEntry) -> bool {
        self.shows_with(entry, &self.filter.to_lowercase())
    }

    /// Brings the panel's copy up to date with the buffer, and rebuilds
    /// the filtered view if anything moved.
    pub(crate) fn sync(&mut self, buffer: &LogBuffer) {
        match buffer.seq_range() {
            None => {
                // Cleared, or nothing logged yet. Either way the copy is
                // wrong and there is nothing to fetch.
                self.entries.clear();
            }
            Some((oldest, newest)) => {
                let held = self.entries.last().map(|entry| entry.seq);
                match held {
                    // A gap: the buffer dropped lines we never saw, or was
                    // cleared and refilled. Nothing to append onto.
                    Some(seq) if seq + 1 < oldest => {
                        self.entries = buffer.snapshot();
                    }
                    Some(seq) if seq >= newest => {}
                    Some(seq) => self.entries.extend(buffer.entries_after(seq)),
                    None => self.entries = buffer.snapshot(),
                }
                // Drop what the buffer has forgotten, so the panel does not
                // keep a log the buffer bounded on purpose.
                let stale = self
                    .entries
                    .iter()
                    .take_while(|entry| entry.seq < oldest)
                    .count();
                self.entries.drain(..stale);
            }
        }

        self.dropped = buffer.dropped();
        self.rebuild_view();
    }

    /// The lines to draw, as indices into the panel's copy.
    pub(crate) fn visible(&self) -> &[usize] {
        &self.visible
    }

    /// The panel's copy of the log.
    pub(crate) fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// How many lines the buffer has dropped.
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }

    fn rebuild_view(&mut self) {
        let key = FilterKey {
            levels: self.levels,
            filter: self.filter.clone(),
            project_only: self.project_only,
            newest: self.entries.last().map(|entry| entry.seq),
        };
        if self.built_for.as_ref() == Some(&key) {
            return;
        }

        let needle = self.filter.to_lowercase();
        // Taken out of `self` for the pass: the filter reads the whole
        // state, and the list being filled is part of it. The `Vec` keeps
        // its allocation across the swap, which is the point of reusing it.
        let mut visible = std::mem::take(&mut self.visible);
        visible.clear();
        visible.extend(
            self.entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| self.shows_with(entry, &needle))
                .map(|(index, _)| index),
        );
        self.visible = visible;
        self.built_for = Some(key);
    }
}

/// Case-insensitive substring search that allocates nothing.
///
/// ASCII case folding: log targets are module paths and messages are
/// English, and the allocation this replaces was per line per frame. A
/// non-ASCII letter matches case-sensitively, which is a smaller surprise
/// than a console that stutters.
fn contains_ignore_case(haystack: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    let (haystack, needle) = (haystack.as_bytes(), needle_lower.as_bytes());
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests;

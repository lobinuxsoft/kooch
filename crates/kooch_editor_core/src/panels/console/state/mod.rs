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

use kooch_core::{LogBuffer, LogEntry};

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
    /// Which visible row the keyboard is on, as an index into
    /// [`visible`](Self::visible).
    ///
    /// `None` until a key or a click puts it somewhere. Stored as a
    /// position in the *filtered* list rather than an entry sequence: the
    /// arrows move through what is on screen, and a line the filter hides
    /// is not somewhere the cursor can be.
    cursor: Option<usize>,
    /// Set when the cursor moves, so the view scrolls to follow it.
    ///
    /// Without this the cursor moved and nothing happened on screen:
    /// `show_rows` only builds the rows in view, so a cursor one row past
    /// the edge was never drawn and never scrolled to. Moving something
    /// invisible is indistinguishable from the key not arriving.
    scroll_to_cursor: bool,
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
            cursor: None,
            scroll_to_cursor: false,
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
    /// The highlighted row, clamped to what is currently visible.
    ///
    /// Clamped on read rather than on write, because the filter can shrink
    /// the list under a cursor that was valid when it was set.
    pub(crate) fn cursor(&self) -> Option<usize> {
        self.cursor
            .filter(|_| !self.visible.is_empty())
            .map(|row| row.min(self.visible.len() - 1))
    }

    /// Moves the cursor by `delta` rows, clamped, starting from the last
    /// row when nothing is highlighted yet.
    ///
    /// Starting at the end is deliberate: a log is read from the bottom,
    /// so the first press of Up should offer the newest line rather than
    /// the oldest one thousands of rows above.
    pub(crate) fn move_cursor(&mut self, delta: isize) {
        if self.visible.is_empty() {
            self.cursor = None;
            return;
        }
        let last = self.visible.len() - 1;
        let from = match self.cursor() {
            Some(row) => row as isize,
            None => last as isize + 1,
        };
        self.cursor = Some((from + delta).clamp(0, last as isize) as usize);
        self.scroll_to_cursor = true;
        // Following the tail would drag the view off whatever is being
        // read; moving the cursor is a statement that the user is reading.
        self.follow = false;
    }

    /// Puts the cursor on the first or last visible row.
    pub(crate) fn cursor_to_edge(&mut self, end: bool) {
        if self.visible.is_empty() {
            self.cursor = None;
            return;
        }
        self.cursor = Some(if end { self.visible.len() - 1 } else { 0 });
        self.scroll_to_cursor = true;
        self.follow = false;
    }

    /// Forgets where the cursor was.
    ///
    /// Called when the panel loses focus: a highlighted line that the
    /// arrows no longer move is a lie about where the keyboard is.
    pub(crate) fn clear_cursor(&mut self) {
        self.cursor = None;
    }

    /// Whether the view should jump to the cursor, clearing the request.
    pub(crate) fn take_scroll_request(&mut self) -> bool {
        std::mem::take(&mut self.scroll_to_cursor)
    }

    /// The text of the highlighted row, for a copy shortcut.
    pub(crate) fn cursor_line(&self) -> Option<&LogEntry> {
        let row = self.cursor()?;
        self.entries.get(*self.visible.get(row)?)
    }

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

#[cfg(test)]
mod cursor_tests {
    use super::*;

    fn with_lines(n: usize) -> ConsoleState {
        let mut state = ConsoleState::default();
        for i in 0..n {
            state.entries.push(LogEntry {
                seq: i as u64,
                level: tracing::Level::INFO,
                target: "test".to_owned(),
                message: format!("line {i}"),
                from_project: false,
            });
            state.visible.push(i);
        }
        state
    }

    /// A log is read from the bottom, so the first Up offers the newest
    /// line rather than the oldest one thousands of rows above.
    #[test]
    fn the_first_step_up_lands_on_the_newest_line() {
        let mut state = with_lines(50);
        state.move_cursor(-1);
        assert_eq!(state.cursor(), Some(49));
    }

    #[test]
    fn the_cursor_stops_at_both_ends() {
        let mut state = with_lines(3);
        state.cursor_to_edge(false);
        state.move_cursor(-10);
        assert_eq!(state.cursor(), Some(0), "walked off the top");
        state.move_cursor(100);
        assert_eq!(state.cursor(), Some(2), "walked off the bottom");
    }

    /// Moving the cursor is a statement that the user is reading, and
    /// following the tail would drag the view out from under them.
    #[test]
    fn moving_the_cursor_stops_following() {
        let mut state = with_lines(5);
        assert!(state.follow, "the default is to follow");
        state.move_cursor(-1);
        assert!(!state.follow);
    }

    /// The filter can shrink the list under a cursor that was valid when
    /// it was set, so the clamp is on read.
    #[test]
    fn a_cursor_past_the_end_is_clamped_not_lost() {
        let mut state = with_lines(10);
        state.cursor_to_edge(true);
        assert_eq!(state.cursor(), Some(9));
        state.visible.truncate(3);
        assert_eq!(
            state.cursor(),
            Some(2),
            "should clamp into the shorter list"
        );
    }

    #[test]
    fn an_empty_log_has_nowhere_for_a_cursor() {
        let mut state = ConsoleState::default();
        state.move_cursor(-1);
        assert_eq!(state.cursor(), None);
        assert!(state.cursor_line().is_none());
    }
}

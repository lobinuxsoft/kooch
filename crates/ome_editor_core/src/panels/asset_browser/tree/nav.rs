//! Keyboard navigation over the asset tree.
//!
//! # Why the rows come from the renderer
//!
//! Moving a cursor through a tree needs a flat, ordered list of what is
//! *visible* — which depends on the collapse state of every folder above
//! a row, and on the search filter. Walking the tree a second time to
//! build that list would mean two walks that must agree about both, and
//! the one that is not on screen is the one that drifts.
//!
//! So the renderer records each row as it draws it, in order, and the
//! keyboard works on the list from the previous frame. The list *is* what
//! was drawn, so it cannot disagree. The cost is one frame of latency on
//! a keypress, which is not perceptible.
//!
//! # Why expanding is a request rather than a call
//!
//! A folder's collapse state lives in egui's memory under an id built
//! with `Ui::make_persistent_id`, which mixes in the id salt of the `Ui`
//! it was called on. That id cannot be reconstructed from outside that
//! `Ui`. So the keyboard leaves a request naming a path, and the renderer
//! applies it when it reaches that folder — where the right id is in
//! scope (#661).

use std::path::{Path, PathBuf};

/// One drawn row, in draw order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AssetRow {
    pub(crate) path: PathBuf,
    /// Whether this row is a folder, and so can expand and collapse.
    pub(crate) is_folder: bool,
    /// Whether it is currently open. Meaningless for a file.
    pub(crate) open: bool,
}

/// The Asset Browser's keyboard state.
#[derive(Default)]
pub(crate) struct AssetNav {
    /// Where the cursor is, by path.
    ///
    /// A path rather than an index: the tree is rebuilt every frame and
    /// rows shift as folders open, so an index means a different row from
    /// one frame to the next.
    pub(crate) cursor: Option<PathBuf>,
    /// Rows drawn last frame, in order.
    pub(crate) rows: Vec<AssetRow>,
    /// A folder the keyboard asked to open or close, applied by the
    /// renderer when it reaches it.
    pub(crate) toggle: Option<(PathBuf, bool)>,
    /// Set when the cursor moves, so the view scrolls to follow it.
    pub(crate) scroll_to_cursor: bool,
    /// The cursor position the selection was last derived from.
    ///
    /// The selection is not stored beside the cursor, it is *derived*
    /// from it — see [`Self::take_cursor_move`].
    last_synced: Option<PathBuf>,
}

impl AssetNav {
    /// The cursor's position in the rows drawn last frame.
    fn index(&self) -> Option<usize> {
        let cursor = self.cursor.as_ref()?;
        self.rows.iter().position(|row| &row.path == cursor)
    }

    /// The row the cursor is on, if it is still on screen.
    pub(crate) fn current(&self) -> Option<&AssetRow> {
        self.index().map(|i| &self.rows[i])
    }

    /// Moves the cursor `delta` rows, clamped to the ends.
    ///
    /// With no cursor yet, or one whose row has gone — a folder closed
    /// over it, a file deleted — this lands on the first row. Silently
    /// doing nothing would look like the key was not received.
    pub(crate) fn step(&mut self, delta: isize) {
        if self.rows.is_empty() {
            self.cursor = None;
            return;
        }
        let last = self.rows.len() as isize - 1;
        let next = match self.index() {
            Some(i) => (i as isize + delta).clamp(0, last),
            None => 0,
        };
        self.cursor = Some(self.rows[next as usize].path.clone());
        self.scroll_to_cursor = true;
    }

    /// Right arrow: open a closed folder, or step into an open one.
    ///
    /// Stepping in rather than doing nothing is what makes the key
    /// repeatable — hold Right and you descend, which is what a tree is
    /// expected to do.
    pub(crate) fn expand_or_enter(&mut self) {
        let Some(row) = self.current().cloned() else {
            self.step(0);
            return;
        };
        match (row.is_folder, row.open) {
            (true, false) => {
                self.toggle = Some((row.path, true));
                self.scroll_to_cursor = true;
            }
            (true, true) => self.step(1),
            // A file has nothing to open and nothing below it that
            // belongs to it.
            (false, _) => {}
        }
    }

    /// Left arrow: close an open folder, or move to the parent.
    ///
    /// The parent is found by path rather than by walking back through the
    /// rows: the row above a deep file can belong to a different branch
    /// entirely, and jumping there would be surprising.
    pub(crate) fn collapse_or_parent(&mut self) {
        let Some(row) = self.current().cloned() else {
            self.step(0);
            return;
        };
        if row.is_folder && row.open {
            self.toggle = Some((row.path, false));
            self.scroll_to_cursor = true;
            return;
        }
        if let Some(parent) = row.path.parent()
            && self.rows.iter().any(|r| r.path == parent)
        {
            self.cursor = Some(parent.to_path_buf());
            self.scroll_to_cursor = true;
        }
    }

    /// Puts the cursor on the first or last drawn row.
    pub(crate) fn to_edge(&mut self, end: bool) {
        let Some(row) = (if end {
            self.rows.last()
        } else {
            self.rows.first()
        }) else {
            return;
        };
        self.cursor = Some(row.path.clone());
        self.scroll_to_cursor = true;
    }

    /// Whether `path` is where the cursor is.
    pub(crate) fn is_cursor(&self, path: &Path) -> bool {
        self.cursor.as_deref() == Some(path)
    }

    /// The row the cursor has just landed on, reported once.
    ///
    /// # Why the selection is derived rather than stored
    ///
    /// The panel used to keep the cursor and the selection as two pieces
    /// of state, written by two different hands: the arrows moved the
    /// cursor, and a click wrote the selection. Two writers meant they
    /// could hold different rows, and drawing both made the panel claim
    /// two selections at once. Deriving one from the other removes the
    /// possibility rather than papering over it — there is nothing left to
    /// keep in agreement.
    ///
    /// Returns `None` when the cursor has not moved, and when it has moved
    /// to nothing. A cleared cursor must not clear the selection: the
    /// cursor is cleared when this panel loses focus, and emptying the
    /// Inspector at that moment would mean clicking into the Inspector
    /// wiped the thing it was about to show.
    pub(crate) fn take_cursor_move(&mut self) -> Option<AssetRow> {
        if self.cursor == self.last_synced {
            return None;
        }
        self.last_synced = self.cursor.clone();
        self.current().cloned()
    }

    /// Takes the pending toggle for `path`, if it is for this folder.
    pub(crate) fn take_toggle_for(&mut self, path: &Path) -> Option<bool> {
        match &self.toggle {
            Some((p, open)) if p == path => {
                let open = *open;
                self.toggle = None;
                Some(open)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: &str, open: bool) -> AssetRow {
        AssetRow {
            path: PathBuf::from(path),
            is_folder: true,
            open,
        }
    }

    fn file(path: &str) -> AssetRow {
        AssetRow {
            path: PathBuf::from(path),
            is_folder: false,
            open: false,
        }
    }

    /// `assets/` open, with two files, then a closed `src/`.
    fn nav() -> AssetNav {
        AssetNav {
            rows: vec![
                folder("/p/assets", true),
                file("/p/assets/a.png"),
                file("/p/assets/b.png"),
                folder("/p/src", false),
            ],
            ..Default::default()
        }
    }

    /// A first press has to land somewhere, or it reads as a key that was
    /// not received.
    #[test]
    fn the_first_step_lands_on_the_first_row() {
        let mut n = nav();
        n.step(1);
        assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/assets")));
    }

    #[test]
    fn stepping_stops_at_both_ends() {
        let mut n = nav();
        n.to_edge(false);
        n.step(-5);
        assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/assets")));
        n.step(50);
        assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/src")));
    }

    #[test]
    fn right_opens_a_closed_folder() {
        let mut n = nav();
        n.cursor = Some(PathBuf::from("/p/src"));
        n.expand_or_enter();
        assert_eq!(n.toggle, Some((PathBuf::from("/p/src"), true)));
    }

    /// Held Right should descend rather than stop, which is what makes it
    /// feel like a tree.
    #[test]
    fn right_on_an_open_folder_steps_into_it() {
        let mut n = nav();
        n.cursor = Some(PathBuf::from("/p/assets"));
        n.expand_or_enter();
        assert_eq!(n.toggle, None, "an open folder has nothing to open");
        assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/assets/a.png")));
    }

    #[test]
    fn left_closes_an_open_folder() {
        let mut n = nav();
        n.cursor = Some(PathBuf::from("/p/assets"));
        n.collapse_or_parent();
        assert_eq!(n.toggle, Some((PathBuf::from("/p/assets"), false)));
    }

    /// From a file, Left goes to the folder that contains it — by path,
    /// because the row above can belong to another branch.
    #[test]
    fn left_on_a_file_goes_to_its_own_parent() {
        let mut n = nav();
        n.cursor = Some(PathBuf::from("/p/assets/b.png"));
        n.collapse_or_parent();
        assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/assets")));
    }

    /// A parent that is not drawn — filtered out, or above a collapsed
    /// root — is not somewhere to jump to.
    #[test]
    fn left_does_not_jump_to_a_parent_that_is_not_on_screen() {
        let mut n = AssetNav {
            rows: vec![file("/p/assets/deep/x.png")],
            cursor: Some(PathBuf::from("/p/assets/deep/x.png")),
            ..Default::default()
        };
        n.collapse_or_parent();
        assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/assets/deep/x.png")));
    }

    /// A folder closing over the cursor, or a deleted file, leaves a path
    /// that no longer has a row.
    #[test]
    fn a_cursor_whose_row_vanished_recovers_on_the_next_key() {
        let mut n = nav();
        n.cursor = Some(PathBuf::from("/p/gone/away.png"));
        assert!(n.current().is_none());
        n.step(1);
        assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/assets")));
    }

    #[test]
    fn a_toggle_is_only_taken_by_the_folder_it_names() {
        let mut n = nav();
        n.toggle = Some((PathBuf::from("/p/src"), true));
        assert_eq!(n.take_toggle_for(Path::new("/p/assets")), None);
        assert_eq!(n.take_toggle_for(Path::new("/p/src")), Some(true));
        assert_eq!(n.take_toggle_for(Path::new("/p/src")), None, "taken once");
    }

    /// The selection follows the cursor however it was moved, and reports
    /// once — a repeat would re-select every frame and fight a click.
    #[test]
    fn a_cursor_move_is_reported_exactly_once() {
        let mut n = nav();
        assert_eq!(n.take_cursor_move(), None, "nothing has moved yet");
        n.step(1);
        assert_eq!(n.take_cursor_move(), Some(folder("/p/assets", true)));
        assert_eq!(n.take_cursor_move(), None, "reported once");
        n.cursor = Some(PathBuf::from("/p/assets/b.png"));
        assert_eq!(n.take_cursor_move(), Some(file("/p/assets/b.png")));
    }

    /// Losing focus clears the cursor. That must not read as "select
    /// nothing", or clicking into the Inspector would empty it.
    #[test]
    fn a_cleared_cursor_does_not_report_a_selection() {
        let mut n = nav();
        n.cursor = Some(PathBuf::from("/p/src"));
        n.take_cursor_move();
        n.cursor = None;
        assert_eq!(n.take_cursor_move(), None);
        // And the same row is selectable again afterwards, rather than
        // being swallowed as "unchanged".
        n.cursor = Some(PathBuf::from("/p/src"));
        assert_eq!(n.take_cursor_move(), Some(folder("/p/src", false)));
    }

    #[test]
    fn an_empty_tree_has_nowhere_to_put_a_cursor() {
        let mut n = AssetNav::default();
        n.step(1);
        n.to_edge(true);
        n.expand_or_enter();
        n.collapse_or_parent();
        assert_eq!(n.cursor, None);
    }
}

//! Keyboard navigation over the asset tree.

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
    pub(crate) cursor: Option<PathBuf>,
    /// Rows drawn last frame, in order.
    pub(crate) rows: Vec<AssetRow>,
    /// A folder the keyboard asked to open or close, applied by the
    /// renderer when it reaches it.
    pub(crate) toggle: Option<(PathBuf, bool)>,
    /// Set when the cursor moves, so the view scrolls to follow it.
    pub(crate) scroll_to_cursor: bool,
    /// The cursor position the selection was last derived from.
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
mod tests;

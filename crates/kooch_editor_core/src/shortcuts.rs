//! The editing chords, and who gets to hear them.
//!
//! # One table, three readers
//!
//! A chord is written down once — its key, its label, the words on its
//! tooltip — and the keyboard, the Edit menu and the World panel's
//! toolbar all read that. The alternative is what the editor had: the
//! handler checked `Key::Z` in one file and the menu wrote the string
//! `"Undo  Ctrl+Z"` in another, which are two claims that a user finds
//! out have diverged by pressing the key.
//!
//! # Who hears them
//!
//! **The World panel and the View, and nobody else.** These edit the
//! entity you have selected, so they belong to the panels that show
//! entities — pressing Ctrl+D with the Console focused should not
//! duplicate something off-screen, and Ctrl+C in the Assets panel is
//! about a file.
//!
//! 🔴 This is a reversal for undo, which was global on purpose and said
//! so: *"Document shortcuts stay global — Ctrl+Z, Ctrl+S, Play are about
//! the project, not about a panel."* True of Ctrl+S, and it stays global.
//! Not true of Ctrl+Z once the same chord means "undo my typing" inside
//! every text field in the Inspector: the reason to gate it is the same
//! reason `handle_keyboard` gates Ctrl+A.

use kooch_ecs::entity::Entity;

use crate::actions::EditorAction;
use crate::history::Document;
use crate::state::EditorTab;

/// An editing command with a keyboard chord.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EditChord {
    Undo,
    Redo,
    Duplicate,
    Copy,
    Paste,
}

/// Every chord, in the order the menu lists them.
pub(crate) const ALL: [EditChord; 5] = [
    EditChord::Undo,
    EditChord::Redo,
    EditChord::Duplicate,
    EditChord::Copy,
    EditChord::Paste,
];

impl EditChord {
    /// The menu entry's text.
    pub fn label(self) -> &'static str {
        match self {
            EditChord::Undo => "Undo",
            EditChord::Redo => "Redo",
            EditChord::Duplicate => "Duplicate",
            EditChord::Copy => "Copy",
            EditChord::Paste => "Paste",
        }
    }

    /// The chord as a person writes it, shown beside every trigger.
    ///
    /// Spelled `Ctrl` on every platform because the modifier read is
    /// `command`, which is Ctrl on Linux and Windows and ⌘ on macOS —
    /// and the editor does not run on macOS.
    pub fn chord(self) -> &'static str {
        match self {
            EditChord::Undo => "Ctrl+Z",
            EditChord::Redo => "Ctrl+Y",
            EditChord::Duplicate => "Ctrl+D",
            EditChord::Copy => "Ctrl+C",
            EditChord::Paste => "Ctrl+V",
        }
    }

    /// What it does, and the part that is not obvious.
    pub fn tooltip(self) -> &'static str {
        match self {
            EditChord::Undo => {
                "Reverse the last edit. With a project open the reversal is sent to the \
                 project, which owns the world — so it is the project's world that goes \
                 back, not this editor's view of it."
            }
            EditChord::Redo => "Apply again what was just undone.",
            EditChord::Duplicate => {
                "Clone the selected entities where they stand, with every component value \
                 preserved. The copies get fresh handles; nothing about the source is touched."
            }
            EditChord::Copy => {
                "Take the selected entities' components and values into the editor's \
                 clipboard. They can be pasted after the originals are deleted — what is \
                 held is the values, not the entities."
            }
            EditChord::Paste => {
                "Build what was copied as new entities, named after their sources. Each \
                 paste is a separate undo step."
            }
        }
    }

    /// The label with its chord written after it, for a menu entry.
    pub fn menu_text(self) -> String {
        format!("{}  {}", self.label(), self.chord())
    }

    fn key(self) -> egui::Key {
        match self {
            EditChord::Undo => egui::Key::Z,
            EditChord::Redo => egui::Key::Y,
            EditChord::Duplicate => egui::Key::D,
            EditChord::Copy => egui::Key::C,
            EditChord::Paste => egui::Key::V,
        }
    }
}

/// Whether a chord is live this frame, and why it might not be.
///
/// 🔴 The two halves of the rule are not the same rule, which is the
/// whole of #813:
///
/// - **Undo and redo** follow the *document*. Any panel that edits
///   something has them — the Inspector on a prefab, the Input Map on
///   its map — and each reaches its own history.
/// - **Duplicate, copy and paste** act on the entity selection, so they
///   belong to the two panels that show entities. Ctrl+D over an input
///   map has nothing to duplicate.
///
/// Typing takes all of them, whichever panel is focused.
pub(crate) fn allowed(
    chord: EditChord,
    focused_tab: Option<EditorTab>,
    document: Option<&Document>,
    text_edit_focused: bool,
) -> bool {
    if text_edit_focused {
        return false;
    }
    match chord {
        EditChord::Undo | EditChord::Redo => document.is_some(),
        _ => matches!(focused_tab, Some(EditorTab::World) | Some(EditorTab::View)),
    }
}

/// What the editor should do about a chord, given what is selected.
///
/// An empty result means the chord had nothing to act on — Ctrl+D with
/// no selection — and is the reason this returns a list rather than one
/// action: duplicating three entities is three actions, and the dispatch
/// layer batches them back into one undo step.
pub(crate) fn actions_for(
    chord: EditChord,
    selected: &[Entity],
    document: Option<&Document>,
) -> Vec<EditorAction> {
    match chord {
        // Undo without a document is a chord pressed over a panel that
        // edits nothing — the Console, the Asset Browser. Doing nothing
        // is the answer, not falling back to the scene.
        EditChord::Undo => document
            .map(|document| vec![EditorAction::Undo(document.clone())])
            .unwrap_or_default(),
        EditChord::Redo => document
            .map(|document| vec![EditorAction::Redo(document.clone())])
            .unwrap_or_default(),
        EditChord::Duplicate => selected
            .iter()
            .copied()
            .map(EditorAction::Duplicate)
            .collect(),
        EditChord::Copy => match selected.is_empty() {
            true => Vec::new(),
            false => vec![EditorAction::CopyEntities(selected.to_vec())],
        },
        EditChord::Paste => vec![EditorAction::PasteEntities],
    }
}

/// Reads this frame's keyboard and queues whatever it asked for.
///
/// Called after the dock has drawn, so `focused_tab` is this frame's
/// answer rather than last frame's.
pub(crate) fn gather(
    ui: &egui::Ui,
    focused_tab: Option<EditorTab>,
    document: Option<&Document>,
    selected: &[Entity],
    actions: &mut Vec<EditorAction>,
) {
    let typing = ui.ctx().text_edit_focused();
    for chord in ALL {
        if !allowed(chord, focused_tab, document, typing) {
            continue;
        }
        let pressed = ui
            .ctx()
            .input(|i| i.modifiers.command && i.key_pressed(chord.key()));
        if pressed {
            actions.extend(actions_for(chord, selected, document));
        }
    }
}

#[cfg(test)]
mod tests;

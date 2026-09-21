//! The editing chords, and who gets to hear them.

use kooch_ecs::entity::Entity;

use crate::actions::EditorAction;
use crate::history::Document;
use crate::state::EditorTab;

/// The chord that toggles Play, as Unity binds it: one chord both ways, so starting and stopping
/// are the same gesture from wherever the hands are.
pub(crate) const PLAY_CHORD: &str = "Ctrl+P";

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
    pub fn chord(self) -> &'static str {
        match self {
            EditChord::Undo => "Ctrl+Z",
            EditChord::Redo => "Ctrl+Y · Ctrl+Shift+Z",
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
        // The active scene, because a chord has no pointer and so names no place. Every gesture
        // that DOES name one — a right click on a scene header, on the panel's empty space — builds
        // its own action with that target instead of coming through here.
        EditChord::Paste => vec![EditorAction::PasteEntities {
            into: crate::actions::SpawnTarget::Active,
        }],
    }
}

/// Reads this frame's keyboard and queues whatever it asked for.
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
        let pressed = ui.ctx().input(|i| pressed(chord, i));
        if pressed {
            actions.extend(actions_for(chord, selected, document));
        }
    }
}

/// Whether `chord` was pressed this frame. Ctrl+Shift+Z is redo, as in every editor that also takes
/// Ctrl+Y — so undo has to refuse the shift, or it would fire on the redo chord.
fn pressed(chord: EditChord, input: &egui::InputState) -> bool {
    let (command, shift) = (input.modifiers.command, input.modifiers.shift);
    match chord {
        EditChord::Undo => command && !shift && input.key_pressed(egui::Key::Z),
        EditChord::Redo => {
            command
                && (input.key_pressed(egui::Key::Y) || (shift && input.key_pressed(egui::Key::Z)))
        }
        _ => command && input.key_pressed(chord.key()),
    }
}

#[cfg(test)]
mod tests;

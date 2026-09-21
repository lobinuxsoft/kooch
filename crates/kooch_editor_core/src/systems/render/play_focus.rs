//! Play and Stop from the keyboard, and the editor following them: playing brings the Game panel to
//! the front with the input and the cursor, stopping brings the Edit View back.

use egui_dock::DockState;

use crate::actions::EditorAction;
use crate::state::EditorTab;

/// Queues Play or Stop when the chord is pressed. Heard while the game has the keyboard too — the
/// editor reads its own window's input before forwarding it, and a stop the game could swallow is
/// a stop that does not exist.
pub(super) fn chord(ui: &egui::Ui, playing: bool, actions: &mut Vec<EditorAction>) {
    if ui.ctx().text_edit_focused() {
        return;
    }
    let pressed = ui.input(|input| input.modifiers.command && input.key_pressed(egui::Key::P));
    if pressed {
        actions.push(toggled(playing));
    }
}

/// What the chord does from here.
pub(crate) fn toggled(playing: bool) -> EditorAction {
    match playing {
        true => EditorAction::Stop,
        false => EditorAction::Play,
    }
}

/// Follows the play state across an edge: the panel that matters comes to the front and takes the
/// keyboard. Answers whether play just started — the moment the cursor goes to the game.
pub(super) fn follow(
    dock: &mut DockState<EditorTab>,
    focused_tab: &mut Option<EditorTab>,
    was_playing: &mut bool,
    playing: bool,
) -> bool {
    let started = playing && !*was_playing;
    let stopped = !playing && *was_playing;
    *was_playing = playing;
    let tab = match (started, stopped) {
        (true, _) => EditorTab::Game,
        (_, true) => EditorTab::View,
        _ => return false,
    };
    // Only a panel that exists is brought forward: one the author closed stays closed, and the
    // input still follows the play state.
    if let Some(path) = dock.find_tab(&tab) {
        let _ = dock.set_active_tab(path);
    }
    *focused_tab = Some(tab);
    started
}

#[cfg(test)]
mod tests;

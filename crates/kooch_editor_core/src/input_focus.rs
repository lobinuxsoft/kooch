//! Who owns input this frame.

use crate::state::EditorTab;

/// The single consumer of input this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputOwner {
    /// Nobody: no panel selected, or a text field has the keyboard.
    #[default]
    None,
    /// The View panel — the editor camera reads orbit / pan / fly.
    ViewCamera,
    /// The Game panel — input is forwarded to the project.
    Game,
}

/// Resource holding this frame's answer. Written once by the UI, read by
/// everyone who needs it.
#[derive(Debug, Clone, Copy, Default)]
pub struct InputFocus {
    owner: InputOwner,
}

impl InputFocus {
    pub fn owner(&self) -> InputOwner {
        self.owner
    }

    /// Whether `who` owns input this frame. The only question consumers
    /// should be asking: they do not get to know *why*, so they cannot
    /// grow their own version of the rule.
    pub fn belongs_to(&self, who: InputOwner) -> bool {
        self.owner == who
    }

    /// Publishes this frame's answer, resolved by the UI — the one place
    /// that knows which panel is focused. Consumers in other stages read
    /// it from here rather than reaching for the dock.
    pub fn set_owner(&mut self, owner: InputOwner) {
        self.owner = owner;
    }
}

/// The rule itself, as a function of its inputs.
pub(crate) fn resolve(focused_tab: Option<EditorTab>, text_edit_focused: bool) -> InputOwner {
    if text_edit_focused {
        return InputOwner::None;
    }
    match focused_tab {
        Some(EditorTab::View) => InputOwner::ViewCamera,
        Some(EditorTab::Game) => InputOwner::Game,
        _ => InputOwner::None,
    }
}

/// The game's last [`kooch_input::CursorMode`] request, sent by the host when it changes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GameCursor {
    pub(crate) wants: kooch_input::CursorMode,
    /// Arrived since the editor last applied it.
    pub(crate) fresh: bool,
}

/// A game that never asks keeps the editor's old rule: a click captures.
impl Default for GameCursor {
    fn default() -> Self {
        Self {
            wants: kooch_input::CursorMode::Captured,
            fresh: false,
        }
    }
}

/// The cursor while the editor plays the game (#1266): the game's request applies when it changes
/// or on a click on the game image — a click on its UI keeps it free, as in a built game. `Esc`,
/// stopping, or another panel taking focus take it back: the editor must always get its cursor.
pub(crate) fn cursor_while_playing(
    was: kooch_input::CursorMode,
    playing: bool,
    game_owns_input: bool,
    escape: bool,
    game: Option<kooch_input::CursorMode>,
) -> kooch_input::CursorMode {
    use kooch_input::CursorMode;
    if !playing || !game_owns_input || escape {
        return CursorMode::Free;
    }
    game.unwrap_or(was)
}

#[cfg(test)]
mod tests;

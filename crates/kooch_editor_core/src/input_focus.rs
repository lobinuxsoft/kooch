//! Who owns input this frame.
//!
//! # Why this exists
//!
//! The answer to "should this key reach me" used to be reconstructed in
//! three places that did not know about each other: the editor camera
//! checked panel focus and hover, the remote input sender checked play
//! state plus focus plus egui, and the raw event chain checked its own
//! consumed flag. Three answers to one question is three chances to
//! disagree — and they did: adding the Game panel silently took the
//! keyboard away from the View.
//!
//! So the question is answered **once**, here, and consumers ask rather
//! than re-derive. Adding a panel that wants input is adding a variant
//! to [`InputOwner`], not a condition in three files.
//!
//! # The rule
//!
//! **The focused panel owns input.** Not "the game owns it while
//! playing", not "the viewport owns it while hovered" — whichever panel
//! you selected is the one that hears you. Play state does not enter
//! into it: a game running in the Game panel and a game running while
//! you have the World panel selected are the same game, and the
//! difference is only where you are looking.
//!
//! One exception: a focused **text field** takes the keyboard from
//! everyone. Typing an entity's name must not also drive the player
//! forward.
//!
//! 🔴 The obvious API for that is a trap. `Context::egui_wants_keyboard_input`
//! is documented as *"egui is currently listening on text input (e.g.
//! typing text in a TextEdit)"* and is implemented as
//! `memory.focused().is_some()` — **any** focused widget, including a
//! button reached with Tab or a combo box that was clicked once. Using it
//! meant the View had no keyboard from editor startup until the first
//! click on Play, because something innocuous held focus the whole time.
//! `Context::text_edit_focused` is the question actually being asked, and
//! it is right there in the same impl block.

use crate::state::EditorTab;

/// The single consumer of input this frame.
///
/// Exactly one, by construction — that is the point. Two owners is the
/// bug this type exists to make unrepresentable.
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
///
/// Separate from the resource so it can be read and tested without a
/// dock, an egui context or a GPU — and so the rule is one expression
/// rather than a trail of early returns across three modules.
pub fn resolve(focused_tab: Option<EditorTab>, text_edit_focused: bool) -> InputOwner {
    if text_edit_focused {
        return InputOwner::None;
    }
    match focused_tab {
        Some(EditorTab::View) => InputOwner::ViewCamera,
        Some(EditorTab::Game) => InputOwner::Game,
        _ => InputOwner::None,
    }
}

#[cfg(test)]
mod tests;

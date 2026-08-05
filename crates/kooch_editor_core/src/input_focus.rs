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
mod tests {
    use super::*;

    #[test]
    fn the_focused_panel_owns_input() {
        assert_eq!(
            resolve(Some(EditorTab::View), false),
            InputOwner::ViewCamera
        );
        assert_eq!(resolve(Some(EditorTab::Game), false), InputOwner::Game);
    }

    #[test]
    fn play_state_is_not_part_of_the_rule() {
        // Deliberately absent from the signature. The Game panel gets
        // input because it is focused, not because something is running;
        // the View keeps its camera while the game plays beside it.
        // Anyone reintroducing play state here has to change the
        // signature, which is the point.
        assert_eq!(
            resolve(Some(EditorTab::View), false),
            InputOwner::ViewCamera
        );
    }

    #[test]
    fn a_panel_that_does_not_take_input_owns_nothing() {
        assert_eq!(resolve(Some(EditorTab::World), false), InputOwner::None);
        assert_eq!(resolve(Some(EditorTab::Inspector), false), InputOwner::None);
        assert_eq!(resolve(None, false), InputOwner::None);
    }

    #[test]
    fn a_focused_text_field_takes_the_keyboard_from_everyone() {
        assert_eq!(resolve(Some(EditorTab::View), true), InputOwner::None);
        assert_eq!(resolve(Some(EditorTab::Game), true), InputOwner::None);
    }

    /// The parameter is `text_edit_focused`, not "some widget has focus".
    ///
    /// Reading it the loose way is what broke the View: the editor opens
    /// with a widget focused, so the camera had no keyboard until the
    /// first Play click moved that focus. The name here is the guard —
    /// a caller passing `egui_wants_keyboard_input` is passing the wrong
    /// question, and the name says so at the call site.
    #[test]
    fn only_a_text_field_takes_it_not_any_focused_widget() {
        assert_eq!(
            resolve(Some(EditorTab::View), false),
            InputOwner::ViewCamera,
            "a focused button is not someone typing",
        );
    }

    #[test]
    fn exactly_one_owner_is_representable() {
        // Not a runtime assertion — a note that the type is the
        // guarantee. Two owners cannot be expressed, so "both the camera
        // and the game moved" cannot happen through this path.
        let focus = InputFocus {
            owner: InputOwner::Game,
        };
        assert!(focus.belongs_to(InputOwner::Game));
        assert!(!focus.belongs_to(InputOwner::ViewCamera));
        assert!(!focus.belongs_to(InputOwner::None));
    }
}

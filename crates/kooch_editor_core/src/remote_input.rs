//! Sending the editor's input to the project it is driving.
//!
//! In remote Play the project simulates **headless** — no window, so no
//! window events, so no key ever reaches it. The keyboard belongs to the
//! editor. Pressing Play and then a key did nothing at all (#710).
//!
//! This closes the loop: the editor reads its own input backend once a
//! frame and posts an [`InputSnapshot`] to the host, where
//! `RemoteInputBackend` turns it back into the `Box<dyn InputBackend>`
//! gameplay reads. Project code is identical to the shipped game's; only
//! who fills the buffer differs.
//!
//! # What it costs, and where it does not
//!
//! One frame of latency: the editor captures on its frame, the host reads
//! it on the next. That is fine for "does my jump work" and **wrong for
//! tuning feel** — a stick curve tuned against remote Play is tuned
//! against a lie. `cargo run -- --game` is direct and is where feel gets
//! tuned.

use kooch_core::resource::Resources;
use kooch_input::{InputBackend, InputSnapshot};
use kooch_remote::protocol::Method;

use crate::remote_session::RemoteState;
use crate::state::EditorOverlay;

/// Reads the editor's input and posts it to the host, while playing.
///
/// Runs in `Stage::PreUpdate`, after the input plugin has pumped the
/// backend in `Stage::Input`, so the snapshot describes this frame.
pub(crate) fn send_input_to_host(resources: &mut Resources) {
    if !should_send(resources) {
        return;
    }

    let Some(backend) = resources.get::<Box<dyn InputBackend>>() else {
        return;
    };
    let snapshot = InputSnapshot::from_backend(backend.as_ref());

    let Some(state) = resources.get_mut::<RemoteState>() else {
        return;
    };
    // Nothing held and nothing moving, and the host already knows: a
    // snapshot per frame of an idle keyboard is a round trip that
    // changes nothing. The *first* idle one still goes, because it is
    // what releases whatever was held when the player let go.
    if snapshot.is_idle() && state.last_input_was_idle {
        return;
    }
    state.last_input_was_idle = snapshot.is_idle();

    let Some(session) = state.session.as_ref().filter(|_| state.is_connected()) else {
        return;
    };
    let payload = match kooch_remote::serde_json::to_value(&snapshot) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(%error, "could not encode the input snapshot");
            return;
        }
    };
    // A host built without the input plugin has no such extension, and
    // says so every frame. Not logged for the same reason the physics
    // overlay does not: a line per frame is not a diagnostic.
    let _ = session.client().call(Method::Extension {
        name: "input.state".to_owned(),
        payload,
    });
}

/// Whether this frame's input belongs to the game.
///
/// One question, asked of [`crate::input_focus`] rather than answered
/// here. This function used to rebuild the rule from play state, panel
/// focus and egui's opinion, which is how it drifted out of step with
/// the editor camera's copy of the same rule.
///
/// Note what is *not* here any more: play state. A game running while
/// the World panel is selected is the same game — where you are looking
/// is what decides, and that decision lives in one place.
fn should_send(resources: &Resources) -> bool {
    resources
        .get::<crate::input_focus::InputFocus>()
        .is_some_and(|focus| focus.belongs_to(crate::input_focus::InputOwner::Game))
}

#[cfg(test)]
mod tests {
    use super::*;

    use kooch_input::{KeyCode, MockInputBackend};

    #[test]
    fn a_world_without_a_remote_state_sends_nothing_and_does_not_panic() {
        let mut resources = Resources::new();
        let backend: Box<dyn InputBackend> = Box::new(MockInputBackend::new());
        resources.insert(backend);

        send_input_to_host(&mut resources);
    }

    #[test]
    fn nothing_is_sent_while_the_project_is_not_playing() {
        let mut resources = Resources::new();
        resources.insert(RemoteState::new());
        assert!(!should_send(&resources));
    }

    /// Playing is no longer enough on its own.
    ///
    /// This test used to assert the opposite, and was right until the
    /// Game panel existed: back then playing meant the whole editor was
    /// the game. Now the game has a panel, and a key only reaches it
    /// once that panel is the one you clicked. Without a `GameView` in
    /// resources — which is every headless test — nothing is focused, so
    /// nothing is sent.
    #[test]
    fn playing_alone_does_not_send_without_the_game_panel_focused() {
        let mut resources = Resources::new();
        let mut state = RemoteState::new();
        state.playing = true;
        resources.insert(state);

        assert!(!should_send(&resources));
    }

    /// The idle gate must not swallow the snapshot that releases a key.
    #[test]
    fn the_first_idle_snapshot_still_goes_out() {
        let mut state = RemoteState::new();
        state.playing = true;
        assert!(
            !state.last_input_was_idle,
            "a fresh session must send its first snapshot, idle or not"
        );

        let mut source = MockInputBackend::new();
        source.press_key(KeyCode::KeyW);
        let held = InputSnapshot::from_backend(&source);
        assert!(!held.is_idle());
    }
}

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

use std::path::Path;

use super::{game_env, parse_launch_env};

fn pairs(raw: &str) -> Vec<(String, String)> {
    parse_launch_env(raw)
}

fn value_of<'a>(env: &'a [(String, std::ffi::OsString)], key: &str) -> Option<&'a str> {
    // Last wins, the same way `Command::env` resolves a repeated key.
    env.iter()
        .rev()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.to_str())
}

#[test]
fn a_line_becomes_pairs() {
    assert_eq!(
        pairs("KOOCH_SHADING_PAD=4   KOOCH_FRAME_METRICS=log"),
        vec![
            ("KOOCH_SHADING_PAD".to_owned(), "4".to_owned()),
            ("KOOCH_FRAME_METRICS".to_owned(), "log".to_owned()),
        ],
    );
}

/// `RUST_LOG=kooch_render=debug` is a real thing somebody types, so the
/// split is at the first `=` and not at the only one.
#[test]
fn a_value_may_contain_equals() {
    assert_eq!(
        pairs("RUST_LOG=kooch_render=debug"),
        vec![("RUST_LOG".to_owned(), "kooch_render=debug".to_owned())],
    );
}

/// A token that is not a pair is dropped rather than guessed at — and
/// the rest of the line still applies, so one typo does not silently
/// cancel a measurement run's other three variables.
#[test]
fn a_bare_token_is_dropped() {
    assert_eq!(
        pairs("novsync KOOCH_SHADING_PAD=4 =orphan"),
        vec![("KOOCH_SHADING_PAD".to_owned(), "4".to_owned())],
    );
    assert!(pairs("").is_empty());
}

/// 🔴 The Console parses the game's output, and cannot if the format is
/// anything else. A launch line naming it must lose.
#[test]
fn the_log_format_cannot_be_overridden() {
    let env = game_env(&pairs("KOOCH_LOG_FORMAT=text"), None, None, false);
    assert_eq!(value_of(&env, "KOOCH_LOG_FORMAT"), Some("json"));
}

/// The editor knows where the engine is and a text field does not.
#[test]
fn the_editor_owns_the_roots() {
    let env = game_env(
        &pairs("KOOCH_ENGINE_ROOT=/tmp/wrong KOOCH_PROJECT_ROOT=/tmp/wrong"),
        Some(Path::new("/engine")),
        Some(Path::new("/project")),
        false,
    );
    assert_eq!(value_of(&env, "KOOCH_ENGINE_ROOT"), Some("/engine"));
    assert_eq!(value_of(&env, "KOOCH_PROJECT_ROOT"), Some("/project"));
}

/// `RUST_LOG` is a default rather than a decision, so a launch line
/// naming it wins — unlike the three above.
#[test]
fn a_launch_line_wins_the_logs() {
    let env = game_env(&pairs("RUST_LOG=debug"), None, None, false);
    assert_eq!(value_of(&env, "RUST_LOG"), Some("debug"));

    let untouched = game_env(&[], None, None, false);
    assert_eq!(value_of(&untouched, "RUST_LOG"), Some("info"));
}

/// An editor started with `RUST_LOG` set hands it down rather than
/// having `info` written over it.
#[test]
fn an_inherited_log_level_survives() {
    let env = game_env(&[], None, None, true);
    assert_eq!(value_of(&env, "RUST_LOG"), None);
}

/// Everything the line asked for that the editor does not own reaches
/// the game — which is the entire point of the field.
#[test]
fn the_rest_of_the_line_survives() {
    let env = game_env(&pairs("KOOCH_SHADING_PAD=4"), None, None, false);
    assert_eq!(value_of(&env, "KOOCH_SHADING_PAD"), Some("4"));
}

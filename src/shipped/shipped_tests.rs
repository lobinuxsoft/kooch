//! #758 — how a shipped game recognises itself.

use super::*;
use kooch_core::asset_loader::shares_for_build;

/// 🔴 The round trip the whole scheme rests on: the packaging step
/// writes this string and the game reads it back. Two files, one format
/// — so the format lives in one of them.
#[test]
fn a_key_survives_the_environment_variable() {
    let key = PackKey::generate();

    assert_eq!(parse_shares(&shares_for_build(&key)), Some(key));
}

/// It arrives through a shell, a cargo invocation and an environment.
#[test]
fn surrounding_whitespace_is_ignored() {
    let key = PackKey::generate();
    let messy = shares_for_build(&key).replace(',', " , ");

    assert_eq!(parse_shares(&messy), Some(key));
}

/// Rather than falling back to the filesystem, where a shipped game has
/// nothing: a black window with no reason is the worst outcome.
#[test]
fn a_malformed_value_is_none() {
    assert_eq!(parse_shares(""), None);
    assert_eq!(parse_shares("nonsense"), None);
    assert_eq!(parse_shares("aa,bb"), None);
}

/// 🔴 The guarantee that keeps development working. This crate is
/// compiled without the variable in every ordinary build, so the game
/// reads the disk exactly as it always did.
#[test]
fn a_development_build_carries_no_key() {
    assert!(
        embedded_key().is_none(),
        "this build has a pack key compiled into it",
    );
    assert!(shipped_pack().is_none());
}

/// A stray `.kpack` in a project directory must not take over a
/// development run, and a key with no pack has nothing to open. Both
/// halves are required, and the test above proves the first is absent
/// here — so this asserts the pair, not either.
#[test]
fn a_pack_alone_is_not_a_shipped_game() {
    let beside = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join(PACK_FILE);
    std::fs::write(&beside, b"not really a pack").unwrap();

    let found = shipped_pack();

    let _ = std::fs::remove_file(&beside);
    assert!(found.is_none(), "a loose pack was taken for a shipped game");
}

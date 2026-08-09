//! Whether this binary is a shipped game, and how it opens its assets
//! (#758).
//!
//! A packaged game is an executable with `scenes/` and `assets.kpack`
//! beside it. Nothing about the build says so — the same binary run from
//! a project directory is a development build — so it is decided by
//! looking: a pack beside the executable, and a key compiled in.
//!
//! # 🔴 The key arrives through the environment at compile time
//!
//! `KOOCH_PACK_SHARES` is read by `option_env!`, so the shares end up in
//! the binary and **nothing is written into the project**. A generated
//! `src/pack_key.rs` would have been the obvious alternative and would
//! sit in a directory that is committed.
//!
//! `build.rs` declares `rerun-if-env-changed` for it, because otherwise
//! cargo would reuse a cached build of this crate and a rebuilt game
//! would carry the previous key — or none.
//!
//! # ⚠️ And it is not thirty-two contiguous bytes
//!
//! The variable carries three XOR shares
//! ([`SplitKey`](kooch_core::asset_loader::SplitKey)), not the key. Each
//! is a one-time pad on its own, so the entropy scan that finds a key in
//! a binary in fifty milliseconds finds nothing.
//!
//! That raises the cost of the automated attack and not the ceiling.
//! Everything a binary can do without help, someone reading it can do —
//! and a game hands its meshes to the GPU in the clear regardless. See
//! `kooch_pack` for the whole of what this is and is not.

use std::path::PathBuf;

use kooch_core::asset_loader::{PackKey, SplitKey};

/// Name the packaging step gives the pack, beside the executable.
pub const PACK_FILE: &str = "assets.kpack";

/// The key this binary was built with, if it was built with one.
///
/// `None` in every development build, which is what makes a `cargo run`
/// read the filesystem exactly as before.
pub fn embedded_key() -> Option<PackKey> {
    parse_shares(option_env!("KOOCH_PACK_SHARES")?)
}

/// Reassembles a key from the comma-separated shares a build carries.
///
/// Split out of [`embedded_key`] so it can be tested: `option_env!`
/// resolves when *this* crate compiles, so a test cannot set it.
fn parse_shares(shares: &str) -> Option<PackKey> {
    let shares: Vec<String> = shares.split(',').map(|s| s.trim().to_owned()).collect();
    match SplitKey::parse(&shares) {
        Some(split) => Some(split.assemble()),
        // Built with a key that will not parse: say so rather than
        // falling back to the filesystem, where a shipped game has
        // nothing. The alternative is a black window and no reason.
        None => {
            tracing::error!(
                target: "kooch::shipped",
                "this build carries a malformed pack key — its assets cannot be opened",
            );
            None
        }
    }
}

/// Formats a key's shares for `KOOCH_PACK_SHARES`.
///
/// The packaging step's half of the contract, kept here so the two ends
/// of one string live in one file. Two copies of a separator is a bug
/// that shows up as a game with no assets.
pub fn shares_for_build(key: &PackKey) -> String {
    SplitKey::split(key).to_hex().join(",")
}

#[cfg(test)]
mod shipped_tests;

/// The pack this game ships with, and the key for it.
///
/// `None` unless there is both a pack beside the executable and a key
/// compiled in. Either alone is not a packaged game: a stray `.kpack` in
/// a project directory must not take over a development run, and a key
/// with no pack has nothing to open.
pub fn shipped_pack() -> Option<(PathBuf, PackKey)> {
    let key = embedded_key()?;
    let beside = std::env::current_exe().ok()?.parent()?.join(PACK_FILE);
    beside.is_file().then_some((beside, key))
}

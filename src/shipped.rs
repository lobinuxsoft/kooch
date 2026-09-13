//! Shipped game = a pack beside the executable plus a key from `KOOCH_PACK_SHARES` via
//! `option_env!` (#758), never written to the project. ⚠️ Three XOR shares
//! ([`SplitKey`](kooch_core::asset_loader::SplitKey)) beat entropy scans, not readers.

use std::path::PathBuf;

use kooch_core::asset_loader::PackKey;

/// Name the packaging step gives the pack, beside the executable.
pub const PACK_FILE: &str = "assets.kpack";

/// The compiled-in key; `None` in development builds, which read the filesystem as before.
pub fn embedded_key() -> Option<PackKey> {
    parse_shares(option_env!("KOOCH_PACK_SHARES")?)
}

/// Reassembles a key from shares, split from [`embedded_key`] to be testable; the format is
/// `kooch_pack`'s.
fn parse_shares(shares: &str) -> Option<PackKey> {
    match kooch_core::asset_loader::key_from_shares(shares) {
        Some(key) => Some(key),
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

#[cfg(test)]
mod shipped_tests;

/// The shipped pack and key — both required: a stray `.kpack` must not take over a dev run, and a
/// key alone opens nothing.
pub fn shipped_pack() -> Option<(PathBuf, PackKey)> {
    let key = embedded_key()?;
    let beside = std::env::current_exe().ok()?.parent()?.join(PACK_FILE);
    beside.is_file().then_some((beside, key))
}

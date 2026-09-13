//! `.kpack` — the container a shipped game reads its assets from (#758): one file of entries, each
//! zstd-compressed and AES-256-GCM encrypted.
//!
//! ```text
//! dist/
//!   mygame              the executable
//!   scenes/             default.scene, and the rest
//!   assets.kpack        this
//! ```
//!
//! ⚠️ A deterrent, not protection: the key has to be in the binary, so an attacker has it too (as
//! with Godot's `.pck`).
//! Not `vach`: it lacks encryption and caps entries at `u16`.

mod key;
mod read;
mod split;
mod write;

pub use key::PackKey;
pub use read::Pack;
pub use split::{SHARES, SHARES_ENV, SplitKey, key_from_shares, shares_for_build};
pub use write::PackWriter;

/// Layout version: a reader refuses one it does not know. Left in the clear so a newer pack reports
/// that, not a wrong key.
pub const FORMAT_VERSION: u16 = 1;

/// Bytes of AES-GCM nonce, per entry.
const NONCE_LEN: usize = 12;

/// zstd level for packing: 19, not 22 — the last levels cost several times the time for about 1%
/// ratio, and decompression speed does not depend on it.
const ZSTD_LEVEL: i32 = 19;

/// Anything that can go wrong reading or writing a pack.
#[derive(Debug)]
pub enum PackError {
    Io(std::io::Error),
    /// Not a `.kpack`, or truncated before the header.
    NotAPack,
    /// Written by a newer format than this build knows.
    Version(u16),
    /// The key is wrong, or the pack was modified — one variant because AES-GCM cannot tell them
    /// apart.
    Corrupt,
    /// No entry under that name.
    NotFound(String),
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::NotAPack => write!(f, "not a Kóoch asset pack"),
            Self::Version(v) => write!(
                f,
                "pack format version {v}, this build reads {FORMAT_VERSION} — \
                 the pack was made by a newer editor",
            ),
            Self::Corrupt => write!(
                f,
                "the pack could not be decrypted: wrong key, or it was modified",
            ),
            Self::NotFound(name) => write!(f, "no entry named {name} in the pack"),
        }
    }
}

impl std::error::Error for PackError {}

impl From<std::io::Error> for PackError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// One file inside a pack, as the index describes it: `stored_len` to read off disk, `plain_len` to
/// allocate before decompressing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Path relative to the pack's root, always with `/` separators, so a
    /// pack built on Windows reads the same on Linux.
    pub name: String,
    /// Where the bytes start in the file.
    pub offset: u64,
    /// How many bytes are there — compressed and encrypted.
    pub stored_len: u64,
    /// How many bytes come back out.
    pub plain_len: u64,
    /// Whether the payload went through zstd; already-compressed files like `.png` skip it.
    pub compressed: bool,
    /// This entry's AES-GCM nonce.
    pub nonce: [u8; NONCE_LEN],
}

#[cfg(test)]
mod tests;

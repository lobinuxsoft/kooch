//! `.kpack` — the container a shipped game reads its assets out of (#758).
//!
//! One file beside the executable holding every asset the game
//! references, each entry compressed with **zstd** and encrypted with
//! **AES-256-GCM**.
//!
//! ```text
//! dist/
//!   mygame              the executable
//!   scenes/             default.scene, and the rest
//!   assets.kpack        this
//! ```
//!
//! # ⚠️ What this is not
//!
//! **The key has to be inside the binary for the binary to open the pack,
//! so an attacker has it too.** Godot does the same thing and
//! `godot-key-extract` pulls the key out of their executables; their own
//! documentation calls it a deterrent rather than protection.
//!
//! What it buys is real but bounded: stealing the assets goes from
//! *dragging a `.glb` into Blender* to *pulling the key out of a binary
//! and understanding this format*. That filters almost everyone. It does
//! not filter someone determined, and nothing does.
//!
//! # Why the parts are what they are
//!
//! - **zstd**, because it wins ratio and decompression speed at the same
//!   time: level 19 approaches LZMA's ratio and decompresses about ten
//!   times faster. Packing happens once and slowly; reading happens
//!   always. `lz4` compresses too poorly and `brotli` decompresses slower
//!   for the same ratio.
//! - **AES-256-GCM**, because it is *authenticated*. A pack somebody
//!   edited fails to open and says so, rather than handing the game
//!   plausible rubbish that surfaces later as a crash in a mesh loader.
//! - **Compress, then encrypt.** The other order compresses nothing:
//!   ciphertext has no structure left to find.
//! - **Per entry, with its own nonce.** Encrypting the file as one blob
//!   would mean decrypting 500 MB to read one texture.
//! - **The index is encrypted too**, so the list of file names — which is
//!   most of what a game is about — is not readable from the outside.
//!
//! # Why the container is ours
//!
//! [`vach`](https://github.com/zeskeertwee/vach) is the crate for exactly
//! this, MIT, modelled on Godot's `.pck`. It was read before this was
//! written and not adopted: its own README lists encryption as *"yet to
//! be implemented"*, its last release was a year ago, and its `CAPACITY`
//! field is a `u16` — 65 535 entries, a ceiling a real game walks into.
//!
//! What is left after taking compression and cryptography from crates is
//! an index and some offsets, which is what this module is.

mod key;
mod read;
mod write;

pub use key::PackKey;
pub use read::Pack;
pub use write::PackWriter;

/// File magic. Eight bytes so the header stays aligned.
pub const MAGIC: [u8; 8] = *b"KOOCHPK\x01";

/// Layout version. Bumped when the header or an entry changes shape; a
/// reader refuses anything it does not know rather than guessing.
pub const FORMAT_VERSION: u16 = 1;

/// Extension a pack carries.
pub const PACK_EXTENSION: &str = "kpack";

/// Bytes of AES-GCM nonce, per entry.
const NONCE_LEN: usize = 12;

/// zstd level used when packing.
///
/// 19, not 22: the last three levels cost several times the packing time
/// for about a percent of ratio, and packing is something a person waits
/// through. Decompression speed does not depend on the level.
const ZSTD_LEVEL: i32 = 19;

/// Anything that can go wrong reading or writing a pack.
#[derive(Debug)]
pub enum PackError {
    Io(std::io::Error),
    /// Not a `.kpack`, or truncated before the header.
    NotAPack,
    /// Written by a newer format than this build knows.
    Version(u16),
    /// The key is wrong, or the pack was modified.
    ///
    /// One variant for both on purpose: AES-GCM cannot tell them apart,
    /// and pretending otherwise would be a guess in an error message.
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

/// One file inside a pack, as the index describes it.
///
/// `stored_len` and `plain_len` are both kept: the first is what to read
/// off disk, the second is what to allocate before decompressing. Without
/// the second, every read grows a buffer as it goes.
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
    /// Whether the payload went through zstd.
    ///
    /// Not everything does: a `.png` is already compressed, and zstd on
    /// top costs time to make it very slightly bigger.
    pub compressed: bool,
    /// This entry's AES-GCM nonce.
    pub nonce: [u8; NONCE_LEN],
}

#[cfg(test)]
mod tests;

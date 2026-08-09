//! The key a pack is sealed with.
//!
//! # 🔴 One key per project, and never inside the editor
//!
//! The editor can open packs — that is how a build gets verified — and it
//! reads the key from the **project it has open**. It must never carry
//! one of its own.
//!
//! If the editor held a global key, a single extraction from one
//! published editor binary would open the packs of *every game ever made
//! with this engine*, forever. Per project, breaking one says nothing
//! about the next.
//!
//! # It does not belong in version control
//!
//! The build preset does — it is configuration. The key is not: a repo
//! that carries it has published it. Godot draws the same line between
//! `export_presets.cfg` and its encryption key.

// `Generate` is crypto-common's, re-exported through aead. Grepped
// from the source: `aead::OsRng` is gone in 0.11, and the docs still
// describe the old shape.
use aes_gcm::aead::Generate;
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Domain separators. One master key must never do two jobs: the tag is
/// public, sitting at byte 0 of every pack, and a construction that let
/// it be derived from the same key that encrypts would be handing out a
/// sample of the cipher's output for free.
const TAG_INFO: &[u8] = b"kooch.pack.tag.v1";
const DATA_INFO: &[u8] = b"kooch.pack.data.v1";

/// A 256-bit key, as text that can be pasted and stored.
///
/// Held as raw bytes and rendered as lowercase hex. Hex rather than
/// base64 so it is unambiguous to read aloud, retype, and diff — a key
/// that gets mangled in transit produces `PackError::Corrupt`, which
/// says nothing about a stray character.
/// 🔴 `ZeroizeOnDrop`: the bytes are wiped when this goes away. A key
/// left in freed memory is a key in a core dump, in a crash report, and
/// in whatever the allocator hands out next.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct PackKey([u8; 32]);

impl PackKey {
    /// A fresh key from the OS entropy source.
    pub fn generate() -> Self {
        Self(<[u8; 32]>::generate())
    }

    /// Parses 64 hex characters.
    ///
    /// Whitespace is ignored, because this arrives pasted.
    pub fn parse(text: &str) -> Option<Self> {
        let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if text.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        Some(Self(bytes))
    }

    /// The key as 64 lowercase hex characters.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Wraps raw bytes. Crate-internal: a key arrives generated or
    /// parsed, and a third way in is a third way to get it wrong.
    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The master bytes, for splitting them apart.
    pub(crate) fn bytes_for_split(&self) -> &[u8; 32] {
        &self.0
    }

    /// The subkey the cipher actually uses.
    ///
    /// Never the master itself: it also has to produce the tag, and one
    /// key doing two jobs is how a public value ends up related to a
    /// secret one.
    pub(crate) fn data_key(&self) -> [u8; 32] {
        self.derive(DATA_INFO)
    }

    /// The eight bytes a pack starts with.
    ///
    /// 🔴 Derived, not a magic string. `KOOCHPK` at byte 0 is a sign
    /// saying what the file is and which tool to write; derived, the file
    /// is bytes to anyone without the key, and still verifiable by anyone
    /// with it.
    ///
    /// The cost is honest and worth naming: "this is not a pack" and
    /// "wrong key" stop being distinguishable, because telling them apart
    /// is exactly the thing being removed.
    pub(crate) fn tag(&self) -> [u8; 8] {
        let full = self.derive(TAG_INFO);
        full[..8].try_into().expect("8 bytes")
    }

    fn derive(&self, info: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        Hkdf::<Sha256>::new(None, &self.0)
            .expand(info, &mut out)
            .expect("32 bytes is a valid HKDF output length");
        out
    }
}

/// 🔴 Never the key itself.
///
/// A key that reaches a log is a key that reaches wherever logs go — a
/// bug report, a screenshot of a terminal, CI output kept for a year. The
/// derive would have put it in every `{:?}` of every struct holding one.
impl std::fmt::Debug for PackKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PackKey(<redacted>)")
    }
}

/// Same reasoning as [`Debug`]: `{}` is how a value ends up in a log line
/// without anybody deciding to put it there.
impl std::fmt::Display for PackKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod key_tests;

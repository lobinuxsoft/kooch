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

/// A 256-bit key, as text that can be pasted and stored.
///
/// Held as raw bytes and rendered as lowercase hex. Hex rather than
/// base64 so it is unambiguous to read aloud, retype, and diff — a key
/// that gets mangled in transit produces `PackError::Corrupt`, which
/// says nothing about a stray character.
#[derive(Clone, PartialEq, Eq)]
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

    pub(crate) fn bytes(&self) -> &[u8; 32] {
        &self.0
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

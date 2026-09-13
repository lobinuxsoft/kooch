//! The key a pack is sealed with.
//! 🔴 One per project, read from the open project, never held by the editor — one extracted global
//! key would open every game. Kept out of version control.

// `Generate` is crypto-common's, re-exported through aead. Grepped
// from the source: `aead::OsRng` is gone in 0.11, and the docs still
// describe the old shape.
use aes_gcm::aead::Generate;
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Domain separators, so the public tag is never derived from the key that encrypts.
const TAG_INFO: &[u8] = b"kooch.pack.tag.v1";
const DATA_INFO: &[u8] = b"kooch.pack.data.v1";

/// A 256-bit key as lowercase hex, unambiguous to read, retype and diff.
/// 🔴 `ZeroizeOnDrop`: a key left in freed memory ends up in core dumps and crash reports.
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

    /// The subkey the cipher uses — never the master, which also produces the public tag.
    pub(crate) fn data_key(&self) -> [u8; 32] {
        self.derive(DATA_INFO)
    }

    /// The eight bytes a pack starts with. 🔴 Derived, not a magic string, so the file announces
    /// nothing without the key — at the cost that not-a-pack and wrong-key look the same.
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

/// 🔴 Never the key itself: a derived `Debug` would put it in every log, bug report and CI output
/// holding one.
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

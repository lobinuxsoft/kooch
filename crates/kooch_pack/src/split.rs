//! The key inside a shipped binary, as XOR shares: each share alone is a one-time pad, and no 32
//! contiguous bytes are the key.
//! Defeats the automated entropy scan, not someone reading the disassembly.

use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::PackKey;

/// How many pieces a key is stored in: three, since two sit as an obvious pair; more is theatre in
/// the same file.
pub const SHARES: usize = 3;

/// A key split into shares that mean nothing apart.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SplitKey([[u8; 32]; SHARES]);

impl SplitKey {
    /// Splits `key`. Each call produces different shares for the same
    /// key, so two builds of one project do not ship the same bytes.
    pub fn split(key: &PackKey) -> Self {
        use aes_gcm::aead::Generate;

        let mut shares = [[0u8; 32]; SHARES];
        for share in shares.iter_mut().take(SHARES - 1) {
            *share = <[u8; 32]>::generate();
        }
        // The last one carries whatever is needed to make the XOR land on
        // the key.
        let mut last = *key.bytes_for_split();
        for share in shares.iter().take(SHARES - 1) {
            for (a, b) in last.iter_mut().zip(share) {
                *a ^= b;
            }
        }
        shares[SHARES - 1] = last;
        Self(shares)
    }

    /// Puts the key back together.
    pub fn assemble(&self) -> PackKey {
        let mut key = [0u8; 32];
        for share in &self.0 {
            for (a, b) in key.iter_mut().zip(share) {
                *a ^= b;
            }
        }
        PackKey::from_bytes(key)
    }

    /// The shares as hex, as separate strings so they are not adjacent in the binary.
    pub fn to_hex(&self) -> [String; SHARES] {
        std::array::from_fn(|i| self.0[i].iter().map(|b| format!("{b:02x}")).collect())
    }

    /// Rebuilds from what [`to_hex`](Self::to_hex) produced.
    pub fn parse(shares: &[String]) -> Option<Self> {
        if shares.len() != SHARES {
            return None;
        }
        let mut out = [[0u8; 32]; SHARES];
        for (slot, text) in out.iter_mut().zip(shares) {
            let key = PackKey::parse(text)?;
            slot.copy_from_slice(key.bytes_for_split());
        }
        Some(Self(out))
    }
}

/// Same reasoning as [`PackKey`]: shares are key material, and `{:?}` is
/// how key material reaches a log without anyone deciding to put it
/// there.
impl std::fmt::Debug for SplitKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SplitKey(<redacted>)")
    }
}

/// Name of the environment variable a build carries its shares in. 🔴 The editor sets it and the
/// game reads it via `option_env!`, so the format lives in one place.
pub const SHARES_ENV: &str = "KOOCH_PACK_SHARES";

/// Formats a key's shares for [`SHARES_ENV`].
pub fn shares_for_build(key: &PackKey) -> String {
    SplitKey::split(key).to_hex().join(",")
}

/// Reassembles a key from [`shares_for_build`]; `None` for anything malformed, rather than falling
/// back to a filesystem a shipped game lacks.
pub fn key_from_shares(value: &str) -> Option<PackKey> {
    let shares: Vec<String> = value.split(',').map(|s| s.trim().to_owned()).collect();
    Some(SplitKey::parse(&shares)?.assemble())
}

#[cfg(test)]
mod split_tests;

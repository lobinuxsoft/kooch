//! The key as it sits inside a shipped binary.
//!
//! # What this does, exactly
//!
//! Splits the key into shares that XOR back together. Every share but the
//! last is random, and the last is the key XOR'd with all of them — which
//! makes each share on its own a **one-time pad**: it carries no
//! information about the key at all. That part is not obfuscation, it is
//! arithmetic.
//!
//! So a shipped binary has no thirty-two contiguous bytes that are the
//! key, and the entropy scan that finds one in fifty milliseconds finds
//! nothing.
//!
//! # What this does not do
//!
//! **All the shares are in the same binary, and so is the code that XORs
//! them.** Someone reading the disassembly gets the key. This raises the
//! cost of the *automated* attack — which is the shape of essentially
//! every attempt — and does not move the ceiling at all.
//!
//! Nothing moves that ceiling. White-box cryptography is the serious
//! attempt at it and it is broken: of 94 entries in the WhibOx CHES 2017
//! contest, every one fell during the competition, the strongest after 28
//! days.
//!
//! # And a reminder about where the real hole is
//!
//! A game hands its meshes and textures to the GPU in the clear, because
//! that is what drawing them means. RenderDoc and Ninja Ripper take them
//! from there without touching this file. Effort spent past this point
//! protects a door beside an open window.

use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::PackKey;

/// How many pieces a key is stored in.
///
/// Three, not two: two shares sit in a binary as an obvious pair, and a
/// third costs 32 bytes. Beyond that it is theatre — the shares are all
/// in the same file whatever their number.
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

    /// The shares as hex, for a generated source file to embed.
    ///
    /// Separate strings, because the point is that they are not adjacent
    /// in the binary.
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

#[cfg(test)]
mod split_tests;

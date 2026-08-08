//! Asset GUID — 128-bit identity persisted in `<asset>.meta` sidecars.
//!
//! Mirrors Unity's asset GUID model: every imported asset gets a
//! random `Guid::v4()` at first import, written to a TOML sidecar that
//! lives alongside the source file. The sidecar is the source of
//! truth — moving or renaming the source file is fine as long as the
//! `.meta` follows it; the GUID stays stable, scene references keep
//! resolving.
//!
//! `Guid` is a thin newtype around [`uuid::Uuid`] so we control the
//! string representation (lowercase hex, no hyphens, matching Unity's
//! editor-side format) and isolate downstream code from the `uuid`
//! crate's API surface.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 128-bit asset identifier. Stable across asset moves/renames as long
/// as the `.meta` sidecar follows the source file.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Guid(Uuid);

impl Guid {
    /// Generates a fresh random GUID. Used by the asset server at first
    /// import when no `.meta` sidecar exists yet.
    pub fn new_v4() -> Self {
        Self(Uuid::new_v4())
    }

    /// Constructs a GUID from raw 128 bits. Reserved for tests and
    /// deterministic fixtures — production paths use [`Guid::new_v4`]
    /// or [`Guid::from_str`] from a `.meta` file.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    /// The canonical 32-character lowercase hex form (no hyphens),
    /// matching Unity's editor-side serialization shape.
    pub fn as_simple(&self) -> String {
        format!("{}", self.0.as_simple())
    }

    /// Underlying [`Uuid`] for interop with crates that take `uuid`
    /// types directly.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Guid({})", self.as_simple())
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.as_simple())
    }
}

impl FromStr for Guid {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

#[cfg(test)]
mod tests;

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
mod tests {
    use super::*;

    #[test]
    fn v4_is_unique_per_call() {
        let a = Guid::new_v4();
        let b = Guid::new_v4();
        assert_ne!(a, b);
    }

    #[test]
    fn display_is_32_lowercase_hex_no_hyphens() {
        let g = Guid::from_bytes([
            0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44,
            0x00, 0x00,
        ]);
        let rendered = g.to_string();
        assert_eq!(rendered, "550e8400e29b41d4a716446655440000");
        assert_eq!(rendered.len(), 32);
        assert!(
            rendered
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn round_trip_via_string() {
        let g = Guid::new_v4();
        let s = g.to_string();
        let parsed: Guid = s.parse().expect("display form should parse");
        assert_eq!(g, parsed);
    }

    #[test]
    fn parses_hyphenated_form_too() {
        // Unity stores no hyphens, but we accept both so users can
        // paste a UUID from any common source.
        let hyphenated = "550e8400-e29b-41d4-a716-446655440000";
        let parsed: Guid = hyphenated.parse().expect("hyphenated form should parse");
        assert_eq!(parsed.to_string(), "550e8400e29b41d4a716446655440000");
    }

    #[test]
    fn rejects_garbage() {
        assert!("not a guid".parse::<Guid>().is_err());
        assert!("".parse::<Guid>().is_err());
    }

    #[test]
    fn serde_round_trip_through_toml() {
        // The `.meta` sidecar serializes through serde + toml. Confirm
        // the transparent newtype renders as a plain string field.
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Wrap {
            guid: Guid,
        }
        let original = Wrap {
            guid: Guid::new_v4(),
        };
        let text = toml::to_string(&original).expect("serializes");
        assert!(text.contains("guid = "));
        assert!(!text.contains("Guid"), "should render as plain string");
        let parsed: Wrap = toml::from_str(&text).expect("deserializes");
        assert_eq!(parsed, original);
    }
}

//! Which source tree a materialised engine came from (#761).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::VendorError;

/// File a materialised engine records its identity in.
pub const STAMP_FILE: &str = ".kooch-engine-stamp";

/// The identity of an engine source tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineStamp {
    /// The editor version that produced this tree.
    pub engine_version: String,
    /// FNV-1a over every file the vendor walk visits — relative path and
    /// contents, in a stable order.
    pub tree_hash: u64,
}

impl EngineStamp {
    /// The stamp `source` should be recorded under.
    pub fn of_source(source: &Path) -> Result<Self, VendorError> {
        match Self::read(source) {
            Some(stamp) => Ok(stamp),
            None => Self::of_tree(source),
        }
    }

    /// Computes the stamp of `source` by reading it.
    pub fn of_tree(source: &Path) -> Result<Self, VendorError> {
        let mut hash = FNV_OFFSET;
        super::copy::walk_engine(source, &mut |rel, abs| {
            // The path is part of the digest: moving a file changes the
            // tree even when not one byte of content does.
            hash = fnv1a(hash, rel.to_string_lossy().replace('\\', "/").as_bytes());
            hash = fnv1a(hash, &fs::read(abs).map_err(VendorError::Io)?);
            Ok(())
        })?;
        Ok(Self {
            engine_version: super::editor_engine_version().to_owned(),
            tree_hash: hash,
        })
    }

    /// Reads the stamp `dir` records, or `None` when it has none.
    pub fn read(dir: &Path) -> Option<Self> {
        let text = fs::read_to_string(Self::path_in(dir)).ok()?;
        ron::from_str(&text).ok()
    }

    /// Writes the stamp into `dir`.
    pub fn write(&self, dir: &Path) -> Result<(), VendorError> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| VendorError::Io(std::io::Error::other(e)))?;
        fs::write(Self::path_in(dir), text).map_err(VendorError::Io)
    }

    /// Whether `dir` still holds the tree its own stamp claims.
    pub fn check(dir: &Path) -> Result<Check, VendorError> {
        let Some(recorded) = Self::read(dir) else {
            return Ok(Check::NoStamp);
        };
        let actual = Self::of_tree(dir)?;
        Ok(match actual.tree_hash == recorded.tree_hash {
            true => Check::Match,
            false => Check::Differs {
                recorded: recorded.tree_hash,
                actual: actual.tree_hash,
            },
        })
    }

    fn path_in(dir: &Path) -> PathBuf {
        dir.join(STAMP_FILE)
    }
}

/// What [`EngineStamp::check`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    /// The tree is what its stamp says.
    Match,
    /// It is not: something was removed, truncated or edited.
    Differs {
        /// What the stamp records.
        recorded: u64,
        /// What the tree hashes to now.
        actual: u64,
    },
    /// Nothing to check against.
    NoStamp,
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a, resumable so a whole tree folds into one value.
fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

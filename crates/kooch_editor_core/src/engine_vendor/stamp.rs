//! Which source tree a materialised engine came from (#761).
//!
//! [`is_engine_source`](super::is_engine_source) asks whether a directory
//! has the *shape* of the engine — `Cargo.toml`, `crates`, `src`. That is
//! the right question when deciding whether a path is usable at all, and
//! the wrong one when deciding whether it is **current**: every copy of
//! the engine ever made passes it.
//!
//! # What went wrong without this
//!
//! The materialised directory is named after `CARGO_PKG_VERSION`, and
//! during development that is `0.1.0` for every build. So a newly
//! installed editor found `~/.local/share/kooch/0.1.0/engine`, saw the
//! three entries, reported it up to date, and every project on the
//! machine went on compiling against source from weeks earlier — with
//! nothing said.
//!
//! 🔴 The [`BuildStamp`](kooch_plugin_api::BuildStamp) does not catch it
//! either: its `engine_hash` is a hash of the same `"0.1.0"` on both
//! sides. The one mechanism built to detect this class of drift is blind
//! to it for as long as the version does not move.
//!
//! # Why the digest is of the source, not of the copy
//!
//! Both, in fact — they are equal by construction. The digest walks the
//! tree with [`walk_engine`](super::copy::walk_engine), the same walk the
//! copy uses, so a source and the copy made from it hash the same. That
//! is what `a_copy_hashes_the_same_as_its_source` pins, and it is what
//! makes the stamp propagate honestly: a packaged editor's `engine/`
//! carries the stamp written when it was packaged, and materialising from
//! it copies that stamp rather than recomputing one.
//!
//! # Why FNV-1a and not a hashing dependency
//!
//! The question is "is this the same tree", not "did somebody forge it".
//! The repo already answers the same class of question this way in
//! `BuildStamp`, which is this mechanism's immediate neighbour. A
//! cryptographic digest would buy resistance to an adversary who is not
//! there, and a dependency to go with it.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::VendorError;

/// File a materialised engine records its identity in.
///
/// Dot-prefixed by the usual convention: it is metadata *about* the
/// directory, not part of the engine, and the walk that decides what the
/// engine is made of skips it by name.
pub const STAMP_FILE: &str = ".kooch-engine-stamp";

/// The identity of an engine source tree.
///
/// Compared whole: the version is not decoration, it is the other half of
/// the answer when a digest collides or a file is read by hand.
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
    ///
    /// Its own if it carries one — a packaged editor's `engine/` does,
    /// written when it was packaged — and otherwise one computed from the
    /// tree. Propagating rather than recomputing is what keeps this cheap
    /// on the path that runs every time a project opens.
    ///
    /// ⚠️ A stamp is believed. Editing a vendored `engine/` in place
    /// leaves it claiming an identity it no longer has; that directory
    /// belongs to the editor, and an installed one is read-only in
    /// practice.
    pub fn of_source(source: &Path) -> Result<Self, VendorError> {
        match Self::read(source) {
            Some(stamp) => Ok(stamp),
            None => Self::of_tree(source),
        }
    }

    /// Computes the stamp of `source` by reading it.
    ///
    /// Walks exactly what would be copied, so this is also the stamp the
    /// resulting copy would compute for itself.
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
    ///
    /// `None` for an unreadable or unparsable file too, and that is the
    /// useful answer: both mean "this directory cannot say what it is",
    /// which is the same as never having said.
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
    ///
    /// Answers a different question from the one
    /// [`ensure_current`](super::ensure_current) asks. That one compares a
    /// *source* against a destination's stamp — "is this editor's engine
    /// the one on disk". This re-reads the destination and compares it
    /// against itself: a file deleted, truncated or edited after the copy
    /// changes nothing about the stamp, so nothing else would ever notice.
    ///
    /// ⚠️ **Reads the whole tree** — 8 MB. Never on the open path; it is
    /// behind `KOOCH_VERIFY_ENGINE`, and repairing is what a mismatch is
    /// good for.
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

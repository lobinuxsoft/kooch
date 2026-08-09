//! Building a pack.
//!
//! Entries are written as they arrive and the index goes last, so packing
//! never holds more than one file in memory — a project's `assets/` is
//! the one thing in a build that has no upper bound.

use std::collections::BTreeSet;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use aes_gcm::aead::{Aead, Generate};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

use crate::{Entry, FORMAT_VERSION, NONCE_LEN, PackError, PackKey, ZSTD_LEVEL};

/// Extensions whose contents are already compressed. Running zstd over
/// them costs time to make the result very slightly larger.
const ALREADY_COMPRESSED: [&str; 9] = [
    "png", "jpg", "jpeg", "webp", "ktx2", "basis", "ogg", "mp3", "kpack",
];

/// Writes a `.kpack`.
///
/// Consumes itself on [`finish`](Self::finish): a pack without its index
/// is unreadable, and a writer that could be dropped half-way would make
/// that a runtime problem instead of a compile-time one.
pub struct PackWriter<W: Write + Seek> {
    out: W,
    cipher: Aes256Gcm,
    tag: [u8; 8],
    entries: Vec<Entry>,
    names: BTreeSet<String>,
    offset: u64,
}

impl<W: Write + Seek> PackWriter<W> {
    /// Starts a pack, reserving the header.
    pub fn new(mut out: W, key: &PackKey) -> Result<Self, PackError> {
        // The header names the index, which is not written yet, so its
        // space is reserved and filled in by `finish`.
        out.write_all(&[0u8; HEADER_LEN])?;
        Ok(Self {
            // The subkey, never the master: the master also derives the
            // tag, which sits in the clear at byte 0.
            cipher: Aes256Gcm::new(&key.data_key().into()),
            tag: key.tag(),
            out,
            entries: Vec::new(),
            names: BTreeSet::new(),
            offset: HEADER_LEN as u64,
        })
    }

    /// Adds `bytes` under `name`.
    ///
    /// `name` is normalised to `/` separators, so a pack built on Windows
    /// reads the same everywhere.
    ///
    /// Adding the same name twice is refused rather than silently keeping
    /// the last: a duplicate means the caller merged two trees that
    /// collide, and finding out at read time — where one of them has
    /// simply vanished — is how a missing texture becomes an afternoon.
    pub fn add(&mut self, name: &str, bytes: &[u8]) -> Result<(), PackError> {
        let name = normalise(name);
        if !self.names.insert(name.clone()) {
            return Err(PackError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("two files claim the name {name} in one pack"),
            )));
        }

        let compressed = worth_compressing(&name);
        let payload = match compressed {
            true => zstd::encode_all(bytes, ZSTD_LEVEL).map_err(PackError::Io)?,
            false => bytes.to_vec(),
        };

        // A nonce reused across entries under one key breaks AES-GCM
        // outright, so each gets its own from the system RNG.
        let nonce = <[u8; NONCE_LEN]>::generate();
        let sealed = self
            .cipher
            .encrypt(&Nonce::from(nonce), payload.as_slice())
            .map_err(|_| PackError::Corrupt)?;

        self.out.write_all(&sealed)?;
        self.entries.push(Entry {
            name,
            offset: self.offset,
            stored_len: sealed.len() as u64,
            plain_len: bytes.len() as u64,
            compressed,
            nonce,
        });
        self.offset += sealed.len() as u64;
        Ok(())
    }

    /// Reads `path` and adds it under `name`.
    pub fn add_file(&mut self, name: &str, path: &Path) -> Result<(), PackError> {
        let bytes = std::fs::read(path)?;
        self.add(name, &bytes)
    }

    /// Writes the index and the header. The pack is only readable after
    /// this returns.
    pub fn finish(mut self) -> Result<W, PackError> {
        // Sorted, so two packs built from the same inputs are the same
        // bytes — a build that differs from itself is a build nobody can
        // check.
        self.entries.sort_by(|a, b| a.name.cmp(&b.name));

        let mut index = Vec::new();
        for entry in &self.entries {
            let name = entry.name.as_bytes();
            index.extend_from_slice(&(name.len() as u16).to_le_bytes());
            index.extend_from_slice(name);
            index.extend_from_slice(&entry.offset.to_le_bytes());
            index.extend_from_slice(&entry.stored_len.to_le_bytes());
            index.extend_from_slice(&entry.plain_len.to_le_bytes());
            index.push(entry.compressed as u8);
            index.extend_from_slice(&entry.nonce);
        }

        // 🔴 The index is sealed too. A plaintext index publishes every
        // file name in the game, which is most of what the game is about
        // — and gives anyone poking at the file the map for free.
        let index_nonce = <[u8; NONCE_LEN]>::generate();
        let index = zstd::encode_all(index.as_slice(), ZSTD_LEVEL).map_err(PackError::Io)?;
        let index = self
            .cipher
            .encrypt(&Nonce::from(index_nonce), index.as_slice())
            .map_err(|_| PackError::Corrupt)?;

        let index_offset = self.offset;
        self.out.write_all(&index)?;

        self.out.seek(SeekFrom::Start(0))?;
        self.out.write_all(&self.tag)?;
        self.out.write_all(&FORMAT_VERSION.to_le_bytes())?;
        self.out
            .write_all(&(self.entries.len() as u32).to_le_bytes())?;
        self.out.write_all(&index_offset.to_le_bytes())?;
        self.out.write_all(&(index.len() as u64).to_le_bytes())?;
        self.out.write_all(&index_nonce)?;
        self.out.flush()?;
        Ok(self.out)
    }
}

/// magic(8) + version(2) + count(4) + index_offset(8) + index_len(8) +
/// index_nonce(12).
///
/// 🔴 `count` is a `u32`, not the `u16` vach uses: 65 535 entries is a
/// ceiling a real project walks into, and a container that silently
/// cannot describe a big game is worse than one that is a little larger.
pub(crate) const HEADER_LEN: usize = 8 + 2 + 4 + 8 + 8 + NONCE_LEN;

/// `\` to `/`, and no leading separator.
fn normalise(name: &str) -> String {
    name.replace('\\', "/").trim_start_matches('/').to_owned()
}

/// Whether zstd is worth running over this name's contents.
fn worth_compressing(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or_default().to_lowercase();
    !ALREADY_COMPRESSED.contains(&ext.as_str())
}

#[cfg(test)]
mod write_tests;

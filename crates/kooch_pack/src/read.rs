//! Reading a pack.
//!
//! The index is decrypted once, at open; entry payloads are read on
//! demand and seeked to directly. A game must never pay for the assets
//! it did not ask for, which is the whole reason this has an index rather
//! than being one compressed stream.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

use crate::write::HEADER_LEN;
use crate::{Entry, FORMAT_VERSION, MAGIC, NONCE_LEN, PackError, PackKey};

/// An open `.kpack`.
///
/// Holds the index in memory (a few dozen bytes per entry) and the file
/// handle. Payloads are never cached here: what to keep resident is the
/// asset system's decision, not the container's.
pub struct Pack<R: Read + Seek> {
    source: R,
    cipher: Aes256Gcm,
    entries: Vec<Entry>,
    by_name: HashMap<String, usize>,
}

impl Pack<BufReader<File>> {
    /// Opens the pack at `path`.
    pub fn open(path: &Path, key: &PackKey) -> Result<Self, PackError> {
        Self::from_reader(BufReader::new(File::open(path)?), key)
    }
}

impl<R: Read + Seek> Pack<R> {
    /// Reads the header and index out of `source`.
    pub fn from_reader(mut source: R, key: &PackKey) -> Result<Self, PackError> {
        let mut header = [0u8; HEADER_LEN];
        source
            .read_exact(&mut header)
            .map_err(|_| PackError::NotAPack)?;
        if header[..8] != MAGIC {
            return Err(PackError::NotAPack);
        }
        let version = u16::from_le_bytes([header[8], header[9]]);
        if version != FORMAT_VERSION {
            return Err(PackError::Version(version));
        }
        let count = u32::from_le_bytes(header[10..14].try_into().expect("4 bytes")) as usize;
        let index_offset = u64::from_le_bytes(header[14..22].try_into().expect("8 bytes"));
        let index_len = u64::from_le_bytes(header[22..30].try_into().expect("8 bytes"));
        let index_nonce: [u8; NONCE_LEN] = header[30..42].try_into().expect("12 bytes");

        let cipher = Aes256Gcm::new(key.bytes().into());
        source.seek(SeekFrom::Start(index_offset))?;
        let mut sealed = vec![0u8; index_len as usize];
        source
            .read_exact(&mut sealed)
            .map_err(|_| PackError::Corrupt)?;
        let index = cipher
            .decrypt(&Nonce::from(index_nonce), sealed.as_slice())
            .map_err(|_| PackError::Corrupt)?;
        let index = zstd::decode_all(index.as_slice()).map_err(|_| PackError::Corrupt)?;

        let entries = parse_index(&index, count)?;
        let by_name = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.name.clone(), i))
            .collect();
        Ok(Self {
            source,
            cipher,
            entries,
            by_name,
        })
    }

    /// Every entry, in the order the pack stores them (sorted by name).
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Whether the pack holds `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(&normalise(name))
    }

    /// Reads one entry.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, PackError> {
        let name = normalise(name);
        let index = *self
            .by_name
            .get(&name)
            .ok_or_else(|| PackError::NotFound(name.clone()))?;
        let entry = self.entries[index].clone();

        self.source.seek(SeekFrom::Start(entry.offset))?;
        let mut sealed = vec![0u8; entry.stored_len as usize];
        self.source
            .read_exact(&mut sealed)
            .map_err(|_| PackError::Corrupt)?;
        let payload = self
            .cipher
            .decrypt(&Nonce::from(entry.nonce), sealed.as_slice())
            .map_err(|_| PackError::Corrupt)?;

        let bytes = match entry.compressed {
            // Sized up front from what the index recorded, rather than
            // letting the buffer grow: the length is known, and a decode
            // that reallocates its way to 40 MB does so several times.
            true => {
                let mut out = Vec::with_capacity(entry.plain_len as usize);
                zstd::stream::copy_decode(payload.as_slice(), &mut out)
                    .map_err(|_| PackError::Corrupt)?;
                out
            }
            false => payload,
        };
        // 🔴 A length that disagrees with the index means the pack and
        // its index describe different things. GCM already proved nobody
        // edited the bytes, so this is a packing bug, and it is worth
        // catching here rather than in whatever loader gets the short
        // buffer.
        if bytes.len() as u64 != entry.plain_len {
            return Err(PackError::Corrupt);
        }
        Ok(bytes)
    }

    /// Reads every entry and checks it comes back whole.
    ///
    /// What the editor's "verify this build" runs. Returns the number of
    /// entries checked, or the first that failed — GCM makes this an
    /// exact answer rather than a heuristic.
    pub fn verify(&mut self) -> Result<usize, PackError> {
        let names: Vec<String> = self.entries.iter().map(|e| e.name.clone()).collect();
        for name in &names {
            self.read(name)?;
        }
        Ok(names.len())
    }
}

/// Walks the decrypted index.
///
/// Every length is checked against what is left rather than trusted: the
/// index is authenticated, so a mismatch here is a bug in the writer, and
/// a panic in a shipped game is a worse way to report one.
fn parse_index(bytes: &[u8], count: usize) -> Result<Vec<Entry>, PackError> {
    let mut entries = Vec::with_capacity(count);
    let mut at = 0usize;
    let take = |n: usize, at: &mut usize| -> Result<&[u8], PackError> {
        let end = at.checked_add(n).ok_or(PackError::Corrupt)?;
        let slice = bytes.get(*at..end).ok_or(PackError::Corrupt)?;
        *at = end;
        Ok(slice)
    };
    for _ in 0..count {
        let name_len = u16::from_le_bytes(take(2, &mut at)?.try_into().expect("2 bytes")) as usize;
        let name = std::str::from_utf8(take(name_len, &mut at)?)
            .map_err(|_| PackError::Corrupt)?
            .to_owned();
        let offset = u64::from_le_bytes(take(8, &mut at)?.try_into().expect("8 bytes"));
        let stored_len = u64::from_le_bytes(take(8, &mut at)?.try_into().expect("8 bytes"));
        let plain_len = u64::from_le_bytes(take(8, &mut at)?.try_into().expect("8 bytes"));
        let compressed = take(1, &mut at)?[0] != 0;
        let nonce: [u8; NONCE_LEN] = take(NONCE_LEN, &mut at)?.try_into().expect("12 bytes");
        entries.push(Entry {
            name,
            offset,
            stored_len,
            plain_len,
            compressed,
            nonce,
        });
    }
    Ok(entries)
}

/// `\` to `/`, and no leading separator — the same normalisation the
/// writer applies, so a lookup finds what `add` stored.
fn normalise(name: &str) -> String {
    name.replace('\\', "/").trim_start_matches('/').to_owned()
}

#[cfg(test)]
mod read_tests;

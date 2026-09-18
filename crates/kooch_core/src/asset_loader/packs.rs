//! Where the [`AssetServer`](super::AssetServer) gets bytes from (#758).

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use kooch_pack::{Pack, PackError, PackKey};

use super::{AssetError, AssetResult};

/// A pack, and the directory its entry names are relative to.
pub(super) struct MountedPack {
    root: PathBuf,
    pack: Pack<BufReader<File>>,
}

impl MountedPack {
    /// Opens `path` and mounts it over `root`.
    pub(super) fn open(root: PathBuf, path: &Path, key: &PackKey) -> Result<Self, PackError> {
        Ok(Self {
            root,
            pack: Pack::open(path, key)?,
        })
    }

    /// How many entries it holds.
    pub(super) fn len(&self) -> usize {
        self.pack.entries().len()
    }

    /// Every entry, as the absolute path the rest of the engine names it
    /// by — the mounted root plus the entry's own name.
    pub(super) fn paths(&self) -> Vec<PathBuf> {
        self.pack
            .entries()
            .iter()
            .map(|entry| self.root.join(&entry.name))
            .collect()
    }

    /// The bytes for `path`, or `None` when this pack does not hold it.
    pub(super) fn read(&mut self, path: &Path) -> Option<Result<Vec<u8>, PackError>> {
        let name = path.strip_prefix(&self.root).ok()?;
        let name = name.to_str()?;
        if !self.pack.contains(name) {
            return None;
        }
        Some(self.pack.read(name))
    }
}

/// The packs a server reads through, in order.
#[derive(Default)]
pub(super) struct Packs(Vec<MountedPack>);

impl Packs {
    pub(super) fn push(&mut self, pack: MountedPack) {
        self.0.push(pack);
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every path every mounted pack holds.
    pub(super) fn paths(&self) -> Vec<PathBuf> {
        self.0.iter().flat_map(MountedPack::paths).collect()
    }

    /// Reads `path` out of a pack, ignoring the disk and reporting a damaged entry as absent — the
    /// caller is asking what the packs hold, and a sidecar that will not decrypt is one this game
    /// cannot use either way.
    pub(super) fn read_packed(&mut self, path: &Path) -> Option<Vec<u8>> {
        self.read(path)?.ok()
    }

    /// Reads `path` out of the first pack that holds it.
    fn read(&mut self, path: &Path) -> Option<Result<Vec<u8>, PackError>> {
        self.0.iter_mut().find_map(|pack| pack.read(path))
    }

    /// The bytes behind `path`: from a mounted pack, or from the disk.
    pub(super) fn read_or_disk(&mut self, path: &Path) -> AssetResult<Vec<u8>> {
        match self.read(path) {
            // 🔴 In a pack and unreadable is an error, not a reason to try the disk. A shipped game
            // has nothing on disk to fall back to, and the error a player would eventually see
            // should say the pack is damaged rather than name a missing file.
            Some(result) => result.map_err(|e| {
                AssetError::Loader(Box::new(std::io::Error::other(format!(
                    "{} could not be read from its pack: {e}",
                    path.display(),
                ))))
            }),
            None => Ok(std::fs::read(path)?),
        }
    }
}

/// Reads a file that belongs to the game: from a mounted pack, or from the disk (#758).
pub fn read_game_file(
    resources: &mut crate::resource::Resources,
    path: &Path,
) -> std::io::Result<Vec<u8>> {
    if let Some(server) = resources.get_mut::<super::AssetServer>()
        && let Some(bytes) = server.read_packed(path)
    {
        return Ok(bytes);
    }
    std::fs::read(path)
}

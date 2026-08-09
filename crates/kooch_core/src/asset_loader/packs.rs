//! Where the [`AssetServer`](super::AssetServer) gets bytes from (#758).
//!
//! In the editor, and in any `cargo run`, that is the filesystem. In a
//! shipped game it is a `.kpack` beside the executable — one file holding
//! every asset, compressed and encrypted.
//!
//! # Packs are tried before the disk
//!
//! Not after. A mounted pack is an explicit decision by whoever built the
//! game, and a release that also happens to have loose files next to it
//! must read the ones it shipped with. Disk-first would also cost a
//! failed `open` syscall for every asset in the common case, since a
//! packaged game has nothing on disk to find.
//!
//! Nothing is mounted by default, so development is unaffected: no packs,
//! no lookups, the disk answers exactly as before.
//!
//! # Names
//!
//! A pack indexes by path relative to the root it was built from. The
//! server works in absolute paths, so a mounted pack remembers its root
//! and the lookup is a `strip_prefix` away. A path outside every mounted
//! root simply is not in a pack, which is the right answer rather than an
//! error — the disk is still there.

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

    /// The bytes for `path`, or `None` when this pack does not hold it.
    ///
    /// 🔴 `None` for "not in this pack" and `Some(Err)` for "in this pack
    /// and unreadable". Collapsing the two would let a corrupt entry fall
    /// through to the disk, where in a shipped game it is not, and the
    /// error the player finally sees would name a missing file rather
    /// than a damaged one.
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

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Reads `path` out of the first pack that holds it.
    fn read(&mut self, path: &Path) -> Option<Result<Vec<u8>, PackError>> {
        self.0.iter_mut().find_map(|pack| pack.read(path))
    }

    /// The bytes behind `path`: from a mounted pack, or from the disk.
    ///
    /// The one place either happens, so a loader added later cannot
    /// accidentally reach past the packs — which in a shipped game means
    /// reaching for a file that is not there.
    ///
    /// A method on `Packs` rather than on the server because the caller
    /// is already holding a borrow of its loaders, and these are
    /// different fields: taking `&mut self` on the server would make two
    /// disjoint borrows look like one.
    pub(super) fn read_or_disk(&mut self, path: &Path) -> AssetResult<Vec<u8>> {
        match self.read(path) {
            // 🔴 In a pack and unreadable is an error, not a reason to
            // try the disk. A shipped game has nothing on disk to fall
            // back to, and the error a player would eventually see should
            // say the pack is damaged rather than name a missing file.
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

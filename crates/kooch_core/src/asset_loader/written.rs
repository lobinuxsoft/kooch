//! The one thing that happens when a file that is an asset gets written.
//!
//! # Why this is not a file watcher
//!
//! Watching the tree means asking the filesystem, every so often, whether
//! anything changed — a question the editor already knows the answer to,
//! because the editor is what wrote the file. Polling turns a fact into a
//! guess, pays for it every frame, and still arrives late.
//!
//! So the trigger is the save itself. Whoever writes an asset says so,
//! once, and that is the only moment any of this runs. There is no
//! background scan and no per-frame cost.
//!
//! It also sidesteps a trap specific to how this project is stored: the
//! repository lives on an NTFS mount through FUSE, where inotify is
//! unreliable and mtime resolution is coarse enough that two saves in the
//! same second can look identical. A watcher would have been subtly
//! broken on the machine it was developed on.
//!
//! # The two halves
//!
//! A save is either the first time a file exists or an edit to one that
//! already did, and those need different things:
//!
//! - **New file** — nothing has loaded it, so there is nothing to
//!   refresh. What it needs is an identity in the [`AssetDatabase`], or
//!   it stays invisible to every picker and every lookup by guid.
//! - **Edited file** — it already has an identity; what it needs is for
//!   the copies already in memory to stop being the old bytes.
//!
//! Both are done here, in that order, because a caller cannot generally
//! tell which case it is in — and asking it to is how one of the two gets
//! forgotten.

use std::path::Path;

use crate::asset_database::{AssetDatabase, AssetEntry};
use crate::asset_loader::AssetServer;
use crate::asset_meta;
use crate::guid::Guid;
use crate::resource::Resources;

/// What [`asset_written`] managed to do.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    /// Identity the file was registered under, when it has a `.meta`
    /// sidecar to take one from.
    pub guid: Option<Guid>,
    /// How many loaded assets were refreshed from the new bytes. Zero
    /// when nothing had loaded this path — the ordinary case for a file
    /// that was just created.
    pub reloaded: usize,
}

/// Registers `path`'s identity and refreshes anything already loaded from
/// it. Call after writing a file the project treats as an asset.
///
/// Best-effort by design: a file with no `.meta` beside it still gets its
/// loaded copies refreshed, and a file that no longer parses keeps its
/// previous contents in memory. Neither is worth failing a save that has
/// already hit the disk.
pub fn asset_written(path: &Path, resources: &mut Resources) -> Written {
    let mut written = Written::default();

    // Identity first: a reload of a brand-new file finds nothing cached,
    // and registering after that would leave the database correct only
    // from the *next* save onwards.
    if let Ok(meta) = asset_meta::read_meta(path) {
        written.guid = Some(meta.guid);
        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if let Some(database) = resources.get_mut::<AssetDatabase>() {
            database.register(
                meta.guid,
                AssetEntry {
                    path: path.to_path_buf(),
                    mtime,
                    type_name: meta.asset_type,
                },
            );
        }
    }

    // Taken out of `resources` because the reload needs both the server
    // and the storage it writes into, and those live side by side.
    let Some(mut server) = resources.remove::<AssetServer>() else {
        return written;
    };
    match server.reload_path(path, resources) {
        Ok(count) => written.reloaded = count,
        Err(e) => tracing::warn!(
            target: "kooch_core::asset_loader",
            path = %path.display(),
            error = %e,
            "asset was written but could not be re-read; keeping the loaded copy",
        ),
    }
    resources.insert(server);
    written
}

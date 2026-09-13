//! The one thing that happens when a file that is an asset gets written.

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

/// Registers `path`'s identity and refreshes anything already loaded from it. Call after writing a
/// file the project treats as an asset.
pub fn asset_written(path: &Path, resources: &mut Resources) -> Written {
    let mut written = Written::default();

    // Identity first: a reload of a brand-new file finds nothing cached,
    // and registering after that would leave the database correct only
    // from the *next* save onwards.
    if let Ok(meta) = asset_meta::read_meta(path) {
        written.guid = Some(meta.guid);
        // Bumped here rather than by each caller: a consumer that derives something from this asset
        // — a block's generated mesh, its collider — has no other way to notice, because a reload
        // overwrites the value under the existing handle on purpose.
        if let Some(mut reloaded) = resources.remove::<super::ReloadedAssets>() {
            reloaded.bump(meta.guid);
            resources.insert(reloaded);
        }
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

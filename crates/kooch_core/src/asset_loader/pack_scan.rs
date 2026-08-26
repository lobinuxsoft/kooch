//! Populating the [`AssetDatabase`] from a mounted pack (#758).
//!
//! # 🔴 Why mounting is not enough
//!
//! The server reading through a pack answers "give me these bytes". It
//! does not answer "which file is GUID `abc`", and that is the question a
//! scene asks about every asset it references.
//!
//! In a project that answer comes from
//! [`scan_directory_adopting`](crate::asset_database::AssetDatabase::scan_directory_adopting),
//! which walks the **filesystem** and reads the `.meta` beside each file.
//! A packaged game has no such directory: the assets and their sidecars
//! are inside the pack. So the scan happens over the pack's own index,
//! and the sidecars are read out of the pack.
//!
//! Without this the game mounts a pack, resolves nothing, and spawns a
//! scene of entities whose meshes and materials are all missing — with
//! the pack working perfectly the whole time.
//!
//! ⚠️ This is why `.meta` files are packed at all. They are not
//! incidental: they are the index from GUID to path, and a packer that
//! left them out would produce exactly the failure above.

use crate::asset_database::{AssetDatabase, AssetEntry};

use super::AssetServer;

/// What a pack scan found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PackScan {
    /// Assets registered with a GUID.
    pub registered: usize,
    /// Entries with no sidecar beside them.
    ///
    /// Not an error on its own — a pack may carry files nothing
    /// references by GUID — but all of them being orphans means the
    /// sidecars did not travel.
    pub orphans: usize,
}

/// Registers every packed asset that carries a `.meta`.
///
/// Reads the sidecars out of the pack itself, which is where they are.
pub fn scan_packs(server: &mut AssetServer, database: &mut AssetDatabase) -> PackScan {
    let mut scan = PackScan::default();
    let paths = server.packed_paths();

    for path in &paths {
        // Sidecars describe assets; they are not assets themselves.
        if path.extension().is_some_and(|e| e == "meta") {
            continue;
        }
        let meta_path = crate::asset_meta::meta_path_for(path);
        let Some(bytes) = server.read_packed(&meta_path) else {
            scan.orphans += 1;
            continue;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            scan.orphans += 1;
            continue;
        };
        // TOML, and through the same parser the disk path uses. A
        // second copy of "what a sidecar looks like" is a second thing
        // to keep in step.
        let Ok(meta) = toml::from_str::<crate::asset_meta::AssetMeta>(text) else {
            scan.orphans += 1;
            continue;
        };

        database.register(
            meta.guid,
            AssetEntry {
                path: path.clone(),
                // A packed file has no independent mtime, and nothing in a
                // shipped game watches for edits — hot reload is an editor
                // concern and the editor reads the disk.
                mtime: std::time::SystemTime::UNIX_EPOCH,
                type_name: meta.asset_type.clone(),
            },
        );
        scan.registered += 1;
    }

    tracing::info!(
        target: "kooch_core::assets",
        registered = scan.registered,
        orphans = scan.orphans,
        "asset pack scan complete",
    );
    // 🔴 Everything orphaned means the sidecars did not travel, and the
    // symptom is a scene that spawns entities with nothing on them. Said
    // here because nothing downstream can tell the difference between
    // "no GUIDs" and "no assets".
    if scan.registered == 0 && scan.orphans > 0 {
        tracing::error!(
            target: "kooch_core::assets",
            orphans = scan.orphans,
            "the pack holds assets and not one `.meta` — every GUID a scene \
             references will fail to resolve. The pack was built without its \
             sidecars.",
        );
    }
    scan
}

#[cfg(test)]
mod pack_scan_tests;

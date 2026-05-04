//! Project-wide asset registry — `Guid ↔ path` bidirectional map.
//!
//! Mirrors Unity's `AssetDatabase`: the engine scans the project's
//! `assets/` tree at startup, reads every `<file>.meta` sidecar, and
//! caches the resulting `(Guid, path, mtime)` triples. From then on,
//! components and scenes that reference a [`Guid`] can resolve it back
//! to a path without touching the filesystem on the hot path.
//!
//! Scope notes:
//! - Watcher / hot-reload integration is deferred — re-scan on startup
//!   is the contract for now.
//! - Per-type import settings (Unity's "Import Settings" panel) live in
//!   the `.meta` schema's TOML body but are not parsed here yet —
//!   [`AssetMeta`] only exposes `guid` until the type-specific needs
//!   surface.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::asset_meta::{AssetMetaError, meta_path_for, read_meta};
use crate::guid::Guid;

/// Bookkeeping for a single asset registered with the database.
#[derive(Debug, Clone)]
pub struct AssetEntry {
    /// Absolute or project-relative path to the source file. The
    /// database stores whatever the caller registers; convention is
    /// project-relative so paths are portable across machines.
    pub path: PathBuf,
    /// Last-modified time of the source file when registered. Used
    /// (later) by hot-reload to detect external edits.
    pub mtime: SystemTime,
}

/// Errors produced while scanning or registering with the database.
#[derive(Debug)]
pub enum AssetDatabaseError {
    /// Filesystem walk failed.
    Io(std::io::Error),
    /// A sidecar parsed but referenced an asset whose source file is
    /// missing. The database refuses to register an orphan entry.
    OrphanSidecar(PathBuf),
    /// Reading or parsing a `.meta` file failed.
    Meta(AssetMetaError),
}

impl std::fmt::Display for AssetDatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "asset database I/O failed: {e}"),
            Self::OrphanSidecar(p) => {
                write!(f, "sidecar at {p:?} has no matching source asset")
            }
            Self::Meta(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AssetDatabaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::OrphanSidecar(_) => None,
            Self::Meta(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for AssetDatabaseError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<AssetMetaError> for AssetDatabaseError {
    fn from(e: AssetMetaError) -> Self {
        Self::Meta(e)
    }
}

/// Summary returned by [`AssetDatabase::scan_directory`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanReport {
    /// Number of `(guid, path)` pairs successfully registered.
    pub registered: usize,
    /// Sidecars whose source file was missing (orphans). Logged but
    /// not registered; the database refuses to point at nothing.
    pub orphans: usize,
    /// Sidecars that already existed in the database (same path, same
    /// GUID). Idempotent re-scan path.
    pub duplicates: usize,
}

/// Bidirectional asset registry. Insert into `Resources` at startup;
/// the asset server consults it for `load_by_guid` resolution.
#[derive(Debug, Default)]
pub struct AssetDatabase {
    by_guid: HashMap<Guid, AssetEntry>,
    by_path: HashMap<PathBuf, Guid>,
}

impl AssetDatabase {
    /// Constructs an empty database.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the entry for `guid`, or `None` if unknown.
    pub fn entry(&self, guid: Guid) -> Option<&AssetEntry> {
        self.by_guid.get(&guid)
    }

    /// Returns the GUID assigned to `path`, or `None` if the path is
    /// not registered. Path lookup is exact — callers must canonicalize
    /// before querying if they need to compare across input forms.
    pub fn guid_for(&self, path: &Path) -> Option<Guid> {
        self.by_path.get(path).copied()
    }

    /// Number of registered assets.
    pub fn len(&self) -> usize {
        self.by_guid.len()
    }

    /// Whether the database has any registered assets.
    pub fn is_empty(&self) -> bool {
        self.by_guid.is_empty()
    }

    /// Registers `(guid, path)` with the database. Idempotent — the
    /// same call repeated is a no-op. Returns `true` if a new entry
    /// was actually added.
    ///
    /// If `path` was previously registered under a *different* GUID
    /// (e.g. its `.meta` was rewritten), the previous binding is
    /// replaced and the old GUID's entry is removed; this keeps the
    /// bidirectional map consistent.
    pub fn register(&mut self, guid: Guid, entry: AssetEntry) -> bool {
        if let Some(existing_guid) = self.by_path.get(&entry.path).copied() {
            if existing_guid == guid {
                return false;
            }
            // Path's GUID changed (manual .meta edit). Replace.
            self.by_guid.remove(&existing_guid);
        }
        self.by_path.insert(entry.path.clone(), guid);
        self.by_guid.insert(guid, entry);
        true
    }

    /// Recursively scans `root`, reading every `<file>.meta` sidecar
    /// it finds, and registers the resulting GUIDs.
    ///
    /// `.meta` files whose source asset is missing become orphans
    /// (counted in [`ScanReport::orphans`], logged at WARN, not
    /// registered). Existing entries with matching `(path, guid)`
    /// count as duplicates and are skipped.
    pub fn scan_directory(&mut self, root: &Path) -> Result<ScanReport, AssetDatabaseError> {
        let mut report = ScanReport::default();
        scan_recursive(root, self, &mut report)?;
        Ok(report)
    }
}

fn scan_recursive(
    dir: &Path,
    db: &mut AssetDatabase,
    report: &mut ScanReport,
) -> Result<(), AssetDatabaseError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            scan_recursive(&path, db, report)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        // Only iterate non-meta files; the meta is read via meta_path_for.
        if path.extension().is_some_and(|e| e == "meta") {
            continue;
        }
        let meta_path = meta_path_for(&path);
        if !meta_path.exists() {
            // No identity yet — load() will generate one when game code
            // first asks for the asset. Scan does not auto-generate.
            continue;
        }
        let meta = read_meta(&path)?;
        let mtime = entry.metadata()?.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let asset_entry = AssetEntry {
            path: path.clone(),
            mtime,
        };

        if let Some(prev) = db.by_guid.get(&meta.guid)
            && prev.path == path
        {
            report.duplicates += 1;
            continue;
        }
        if db.register(meta.guid, asset_entry) {
            report.registered += 1;
        } else {
            report.duplicates += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_meta::{AssetMeta, write_meta};
    use std::io::Write;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new(name: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "ome_asset_db_{name}_{}",
                std::process::id(),
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        let mut f = fs::File::create(path).expect("create source");
        f.write_all(b"placeholder").expect("write");
    }

    #[test]
    fn empty_dir_scans_to_zero() {
        let dir = TempDir::new("empty");
        let mut db = AssetDatabase::new();
        let report = db.scan_directory(&dir.path).expect("scan");
        assert_eq!(report, ScanReport::default());
        assert!(db.is_empty());
    }

    #[test]
    fn missing_dir_is_no_op() {
        let mut db = AssetDatabase::new();
        let report = db
            .scan_directory(Path::new("/nonexistent/path/xyz"))
            .expect("scan tolerates missing");
        assert_eq!(report.registered, 0);
    }

    #[test]
    fn asset_with_sidecar_is_registered() {
        let dir = TempDir::new("with_sidecar");
        let asset = dir.path.join("foo.glb");
        touch(&asset);
        let meta = AssetMeta::new();
        let expected_guid = meta.guid;
        write_meta(&asset, &meta).expect("write meta");

        let mut db = AssetDatabase::new();
        let report = db.scan_directory(&dir.path).expect("scan");
        assert_eq!(report.registered, 1);
        assert_eq!(report.orphans, 0);
        assert_eq!(db.len(), 1);

        let entry = db.entry(expected_guid).expect("guid registered");
        assert_eq!(entry.path, asset);
        assert_eq!(db.guid_for(&asset), Some(expected_guid));
    }

    #[test]
    fn asset_without_sidecar_is_skipped() {
        let dir = TempDir::new("no_sidecar");
        let asset = dir.path.join("bare.glb");
        touch(&asset);

        let mut db = AssetDatabase::new();
        let report = db.scan_directory(&dir.path).expect("scan");
        assert_eq!(report.registered, 0);
        assert!(db.is_empty());
    }

    #[test]
    fn nested_directories_are_walked() {
        let dir = TempDir::new("nested");
        for sub in ["meshes", "textures", "audio/sfx"] {
            let asset = dir.path.join(sub).join("a.glb");
            touch(&asset);
            write_meta(&asset, &AssetMeta::new()).expect("write meta");
        }

        let mut db = AssetDatabase::new();
        let report = db.scan_directory(&dir.path).expect("scan");
        assert_eq!(report.registered, 3);
        assert_eq!(db.len(), 3);
    }

    #[test]
    fn rescan_is_idempotent() {
        let dir = TempDir::new("rescan");
        let asset = dir.path.join("foo.glb");
        touch(&asset);
        write_meta(&asset, &AssetMeta::new()).expect("write meta");

        let mut db = AssetDatabase::new();
        db.scan_directory(&dir.path).expect("first scan");
        let report = db.scan_directory(&dir.path).expect("second scan");
        assert_eq!(report.registered, 0);
        assert_eq!(report.duplicates, 1);
        assert_eq!(db.len(), 1);
    }

    #[test]
    fn register_replaces_when_path_guid_changes() {
        let mut db = AssetDatabase::new();
        let path = PathBuf::from("foo.glb");
        let g1 = Guid::new_v4();
        let g2 = Guid::new_v4();
        db.register(
            g1,
            AssetEntry {
                path: path.clone(),
                mtime: SystemTime::UNIX_EPOCH,
            },
        );
        db.register(
            g2,
            AssetEntry {
                path: path.clone(),
                mtime: SystemTime::UNIX_EPOCH,
            },
        );
        assert_eq!(db.len(), 1);
        assert_eq!(db.guid_for(&path), Some(g2));
        assert!(db.entry(g1).is_none(), "old GUID should be evicted");
        assert!(db.entry(g2).is_some());
    }
}

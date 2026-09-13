use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::guid::Guid;

use super::entry::AssetEntry;
use super::error::AssetDatabaseError;
use super::report::ScanReport;
use super::scan::scan_recursive;

/// Bidirectional asset registry. Insert into `Resources` at startup;
/// the asset server consults it for `load_by_guid` resolution.
#[derive(Debug, Default)]
pub struct AssetDatabase {
    pub(super) by_guid: HashMap<Guid, AssetEntry>,
    pub(super) by_path: HashMap<PathBuf, Guid>,
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

    /// Iterates `(path, guid)` pairs across every registered asset.
    /// Used by editor-side snapshot collectors that need to walk the
    /// whole database once per frame.
    pub fn path_iter(&self) -> impl Iterator<Item = (&Path, Guid)> + '_ {
        self.by_path.iter().map(|(p, g)| (p.as_path(), *g))
    }

    /// Iterates `(Guid, &AssetEntry)` pairs whose `type_name` matches `name`. Used by the
    /// inspector's asset picker to populate the dropdown for a typed `AssetRef` field. Order is
    /// unspecified — callers that need a stable presentation should collect + sort.
    pub fn entries_of_type<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = (Guid, &'a AssetEntry)> + 'a {
        self.by_guid
            .iter()
            .filter(move |(_, entry)| entry.type_name.as_deref() == Some(name))
            .map(|(guid, entry)| (*guid, entry))
    }

    /// Registers `(guid, path)` with the database. Idempotent on the path↔GUID mapping; returns
    /// `true` if a brand-new entry was added.
    pub fn register(&mut self, guid: Guid, entry: AssetEntry) -> bool {
        if let Some(existing_guid) = self.by_path.get(&entry.path).copied() {
            if existing_guid == guid {
                if let Some(existing) = self.by_guid.get_mut(&guid) {
                    if existing.type_name.is_none() && entry.type_name.is_some() {
                        existing.type_name = entry.type_name;
                    }
                    if entry.mtime > existing.mtime {
                        existing.mtime = entry.mtime;
                    }
                }
                return false;
            }
            // Path's GUID changed (manual .meta edit). Replace.
            self.by_guid.remove(&existing_guid);
        }
        self.by_path.insert(entry.path.clone(), guid);
        self.by_guid.insert(guid, entry);
        true
    }

    /// Removes the entry for `path` from both maps, returning its GUID if it was registered.
    pub fn remove_path(&mut self, path: &Path) -> Option<Guid> {
        let guid = self.by_path.remove(path)?;
        self.by_guid.remove(&guid);
        Some(guid)
    }

    /// Recursively scans `root`, reading every `<file>.meta` sidecar it finds, and registers the
    /// resulting GUIDs.
    pub fn scan_directory(&mut self, root: &Path) -> Result<ScanReport, AssetDatabaseError> {
        self.scan_directory_adopting(root, &[])
    }

    /// Scans, and **adopts** files with no `.meta` whose extension a registered loader claims.
    pub fn scan_directory_adopting(
        &mut self,
        root: &Path,
        known: &[(&'static str, &'static str)],
    ) -> Result<ScanReport, AssetDatabaseError> {
        let mut report = ScanReport::default();
        scan_recursive(root, self, &mut report, known)?;
        Ok(report)
    }
}

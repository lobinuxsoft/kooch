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

    /// Iterates `(Guid, &AssetEntry)` pairs whose `type_name` matches
    /// `name`. Used by the inspector's asset picker to populate the
    /// dropdown for a typed `AssetRef` field. Order is unspecified —
    /// callers that need a stable presentation should collect + sort.
    pub fn entries_of_type<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = (Guid, &'a AssetEntry)> + 'a {
        self.by_guid
            .iter()
            .filter(move |(_, entry)| entry.type_name.as_deref() == Some(name))
            .map(|(guid, entry)| (*guid, entry))
    }

    /// Sets `type_name` on an existing entry. Returns `true` if the
    /// entry was found and the type was updated (or already matched);
    /// `false` if the GUID is unknown. Idempotent: writing the same
    /// type twice is a no-op.
    pub fn set_type_name(&mut self, guid: Guid, type_name: &str) -> bool {
        let Some(entry) = self.by_guid.get_mut(&guid) else {
            return false;
        };
        match entry.type_name.as_deref() {
            Some(existing) if existing == type_name => true,
            _ => {
                entry.type_name = Some(type_name.to_owned());
                true
            }
        }
    }

    /// Registers `(guid, path)` with the database. Idempotent on the
    /// path↔GUID mapping; returns `true` if a brand-new entry was
    /// added.
    ///
    /// If `path` was previously registered under a *different* GUID
    /// (e.g. its `.meta` was rewritten), the previous binding is
    /// replaced and the old GUID's entry is removed; this keeps the
    /// bidirectional map consistent.
    ///
    /// When the path↔GUID pair already exists, the stored entry's
    /// **`type_name` and `mtime` are upgraded if the incoming entry
    /// carries fresher data**:
    /// - `type_name`: prefer `Some` over `None` (initial scans see
    ///   sidecars before any `load::<T>` and leave the type unknown;
    ///   the first typed load fills it in — we must not lose that).
    /// - `mtime`: take the newer timestamp.
    /// The `path` itself is immutable for an existing entry and is
    /// not overwritten.
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

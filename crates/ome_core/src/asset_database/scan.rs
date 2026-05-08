use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::asset_meta::{meta_path_for, read_meta};

use super::database::AssetDatabase;
use super::entry::AssetEntry;
use super::error::AssetDatabaseError;
use super::report::ScanReport;

pub(super) fn scan_recursive(
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
            type_name: meta.asset_type.clone(),
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

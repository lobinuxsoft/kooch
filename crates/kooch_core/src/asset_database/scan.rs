use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::asset_meta::{meta_path_for, read_meta, read_or_create_typed};

use super::database::AssetDatabase;
use super::entry::AssetEntry;
use super::error::AssetDatabaseError;
use super::report::ScanReport;

/// `(extension, asset type name)` for every loader the app registered.
pub(super) type KnownExtensions<'a> = &'a [(&'static str, &'static str)];

pub(super) fn scan_recursive(
    dir: &Path,
    db: &mut AssetDatabase,
    report: &mut ScanReport,
    known: KnownExtensions<'_>,
) -> Result<(), AssetDatabaseError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            scan_recursive(&path, db, report, known)?;
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
            // 🔴 No identity yet.
            let Some(type_name) = known_type_for(&path, known) else {
                continue;
            };
            match read_or_create_typed(&path, type_name) {
                Ok(_) => {
                    report.adopted += 1;
                    tracing::debug!(
                        target: "kooch_core::asset_database",
                        path = %path.display(),
                        %type_name,
                        "adopted an asset file that had no .meta",
                    );
                }
                Err(e) => {
                    // A read-only asset directory is a legitimate
                    // setup; the file stays unregistered rather than
                    // failing the whole scan.
                    tracing::warn!(
                        target: "kooch_core::asset_database",
                        path = %path.display(),
                        error = %e,
                        "could not create .meta; the file stays unregistered",
                    );
                    continue;
                }
            }
        }
        let meta = read_meta(&path)?;
        let mtime = entry
            .metadata()?
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH);
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

/// The asset type a registered loader claims for this file's extension.
fn known_type_for(path: &Path, known: KnownExtensions<'_>) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    known
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(&ext))
        .map(|(_, type_name)| *type_name)
}

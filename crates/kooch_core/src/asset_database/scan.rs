use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::asset_meta::{meta_path_for, read_meta, read_or_create_typed};

use super::database::AssetDatabase;
use super::entry::AssetEntry;
use super::error::AssetDatabaseError;
use super::report::ScanReport;

/// `(extension, asset type name)` for every loader the app registered.
///
/// Empty means "adopt nothing": a scan with no loaders behaves exactly
/// as it did before adoption existed, which is what the tests that
/// predate it assume.
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
            // 🔴 No identity yet. This used to `continue`, on the
            // reasoning that `load()` mints one when game code first
            // asks — which is circular for anything the editor is
            // supposed to *show*: the browser lists what the database
            // registered, the database registers what has a `.meta`,
            // and the `.meta` appears when something loads the asset.
            // A file written by hand, by a script, or by another tool
            // was therefore invisible forever, and `docs/MEMORY.md`
            // recorded that twice without it being fixed.
            //
            // So: adopt it, if a registered loader claims the
            // extension. Unknown files are still skipped — a README
            // beside a mesh is not an asset — and the type comes from
            // the loader rather than from a guess, so the entry lands
            // fully typed and the Inspector can resolve it.
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
///
/// Case-insensitive, because `.PNG` off a camera and `.png` off a
/// download are the same asset and only one of them would register.
fn known_type_for(path: &Path, known: KnownExtensions<'_>) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    known
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(&ext))
        .map(|(_, type_name)| *type_name)
}

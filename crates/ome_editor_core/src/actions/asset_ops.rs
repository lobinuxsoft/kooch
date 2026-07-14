//! File-system asset operations behind the Asset Browser context menu:
//! create folder / material, rename, duplicate, delete, reveal.
//!
//! Each mutates the project's `assets/` tree on disk and, where needed,
//! drops stale [`AssetDatabase`] bindings and forces a re-scan so the
//! browser + pickers reflect the change next frame. None are undoable.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use ome_core::asset_database::AssetDatabase;
use ome_core::resource::Resources;
use ome_render::material::Material;

use super::EditorAction;
use crate::systems::LastScannedProject;

/// Dispatches an Asset Browser file operation. Returns `true` if it
/// handled `action`, `false` otherwise (so the caller can keep matching).
pub(super) fn handle_asset_op(action: &EditorAction, resources: &mut Resources) -> bool {
    match action {
        EditorAction::CreateFolder { parent, name } => create_folder(parent, name),
        EditorAction::CreateMaterial { folder, name } => create_material(resources, folder, name),
        EditorAction::RenameAsset { path, new_name } => rename_asset(resources, path, new_name),
        EditorAction::RenameFolder { path, new_name } => rename_folder(resources, path, new_name),
        EditorAction::DuplicateAsset { path } => duplicate_asset(resources, path),
        EditorAction::DeleteAsset { path } => delete_asset(resources, path),
        EditorAction::DeleteFolder { path } => delete_folder(resources, path),
        EditorAction::RevealInFileManager { path } => reveal(path),
        EditorAction::OpenInIde { root, file } => open_in_ide(root, file),
        _ => return false,
    }
    true
}

/// Opens `file` in an external IDE with `root` as the workspace folder.
/// Tries `$OME_IDE` (if set), then `codium`, then `code`; falls back to
/// `xdg-open` on the file when no IDE is found.
fn open_in_ide(root: &Path, file: &Path) {
    let configured = std::env::var("OME_IDE").ok();
    let candidates: Vec<&str> = match configured.as_deref() {
        Some(cmd) => vec![cmd],
        None => vec!["codium", "code"],
    };
    for cmd in candidates {
        // `<ide> <workspace> -g <file>` opens the folder + reveals the file.
        if std::process::Command::new(cmd)
            .arg(root)
            .arg("-g")
            .arg(file)
            .spawn()
            .is_ok()
        {
            tracing::info!(ide = cmd, file = %file.display(), "opened in IDE");
            return;
        }
    }
    tracing::warn!("no IDE found (set OME_IDE, or install codium/code); using xdg-open");
    let _ = std::process::Command::new("xdg-open").arg(file).spawn();
}

fn create_folder(parent: &Path, name: &str) {
    let dir = unique_target(parent, OsStr::new(name));
    match std::fs::create_dir_all(&dir) {
        // A fresh empty folder needs no re-scan — the tree walks the
        // filesystem, so it appears next frame on its own.
        Ok(()) => tracing::info!(dir = %dir.display(), "folder created"),
        Err(e) => tracing::error!(dir = %dir.display(), error = %e, "failed to create folder"),
    }
}

fn create_material(resources: &mut Resources, folder: &Path, name: &str) {
    let file = unique_target(folder, OsStr::new(&format!("{name}.ron")));
    let text =
        match ron::ser::to_string_pretty(&Material::default(), ron::ser::PrettyConfig::default()) {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = %e, "failed to serialise new material");
                return;
            }
        };
    match std::fs::write(&file, text) {
        Ok(()) => {
            tracing::info!(file = %file.display(), "material created");
            // Re-scan so eager import writes a `.meta` (fresh GUID) and
            // registers it as a typed asset.
            force_rescan(resources);
        }
        Err(e) => tracing::error!(file = %file.display(), error = %e, "failed to write material"),
    }
}

fn rename_asset(resources: &mut Resources, path: &Path, new_name: &str) {
    let Some(parent) = path.parent() else { return };
    let dest = parent.join(new_name);
    if dest == path {
        return;
    }
    if dest.exists() {
        tracing::warn!(dest = %dest.display(), "rename target exists; skipped");
        return;
    }
    if let Err(e) = std::fs::rename(path, &dest) {
        tracing::error!(from = %path.display(), error = %e, "rename failed");
        return;
    }
    // Move the sidecar alongside so the GUID (and every reference to it)
    // survives the rename.
    let (meta_old, meta_new) = (meta_path(path), meta_path(&dest));
    if meta_old.exists() {
        let _ = std::fs::rename(&meta_old, &meta_new);
    }
    if let Some(db) = resources.get_mut::<AssetDatabase>() {
        db.remove_path(path);
    }
    tracing::info!(from = %path.display(), to = %dest.display(), "asset renamed");
    force_rescan(resources);
}

fn rename_folder(resources: &mut Resources, path: &Path, new_name: &str) {
    let Some(parent) = path.parent() else { return };
    let dest = parent.join(new_name);
    if dest == path {
        return;
    }
    if dest.exists() {
        tracing::warn!(dest = %dest.display(), "rename target exists; skipped");
        return;
    }
    if let Err(e) = std::fs::rename(path, &dest) {
        tracing::error!(from = %path.display(), error = %e, "folder rename failed");
        return;
    }
    // Every asset under the old path is now stale in the database; drop
    // them and let the re-scan re-register under the new paths (the
    // `.meta` sidecars moved with the folder, so GUIDs are preserved).
    prune_db_under(resources, path);
    tracing::info!(from = %path.display(), to = %dest.display(), "folder renamed");
    force_rescan(resources);
}

fn duplicate_asset(resources: &mut Resources, path: &Path) {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let dest = unique_target(parent, name);
    match std::fs::copy(path, &dest) {
        Ok(_) => {
            // Intentionally do NOT copy the `.meta`: the re-scan's eager
            // import assigns the copy a fresh GUID so it is a distinct
            // asset, not an alias of the original.
            tracing::info!(from = %path.display(), to = %dest.display(), "asset duplicated");
            force_rescan(resources);
        }
        Err(e) => tracing::error!(from = %path.display(), error = %e, "duplicate failed"),
    }
}

fn delete_asset(resources: &mut Resources, path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        tracing::error!(path = %path.display(), error = %e, "delete failed");
        return;
    }
    let meta = meta_path(path);
    if meta.exists() {
        let _ = std::fs::remove_file(&meta);
    }
    // Drop the binding so the catalog (rebuilt each frame) stops listing
    // it — no re-scan needed, scans never prune.
    if let Some(db) = resources.get_mut::<AssetDatabase>() {
        db.remove_path(path);
    }
    tracing::info!(path = %path.display(), "asset deleted");
}

fn delete_folder(resources: &mut Resources, path: &Path) {
    if let Err(e) = std::fs::remove_dir_all(path) {
        tracing::error!(path = %path.display(), error = %e, "folder delete failed");
        return;
    }
    prune_db_under(resources, path);
    tracing::info!(path = %path.display(), "folder deleted");
}

fn reveal(path: &Path) {
    let target = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    if let Err(e) = std::process::Command::new("xdg-open").arg(target).spawn() {
        tracing::error!(path = %target.display(), error = %e, "xdg-open failed");
    }
}

/// Removes every database entry whose path lives under `dir`.
fn prune_db_under(resources: &mut Resources, dir: &Path) {
    let Some(db) = resources.get_mut::<AssetDatabase>() else {
        return;
    };
    let stale: Vec<PathBuf> = db
        .path_iter()
        .filter(|(p, _)| p.starts_with(dir))
        .map(|(p, _)| p.to_path_buf())
        .collect();
    for p in stale {
        db.remove_path(&p);
    }
}

/// Forces `scan_project_assets_system` to re-run the full project scan +
/// eager import next frame by clearing its "already scanned" marker.
pub(super) fn force_rescan(resources: &mut Resources) {
    if let Some(last) = resources.get_mut::<LastScannedProject>() {
        last.root = None;
    }
}

/// Returns a non-colliding path in `dir` for `name`, appending `_1`,
/// `_2`, … before the extension if the file already exists.
pub(super) fn unique_target(dir: &Path, name: &OsStr) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("asset");
    let ext = path.extension().and_then(|s| s.to_str());
    for n in 1.. {
        let fname = match ext {
            Some(ext) => format!("{stem}_{n}.{ext}"),
            None => format!("{stem}_{n}"),
        };
        let candidate = dir.join(fname);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("infinite unique-name loop")
}

/// The `<file>.meta` sidecar path for an asset file.
fn meta_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(".meta");
    PathBuf::from(s)
}

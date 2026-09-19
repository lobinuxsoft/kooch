//! File-system asset operations behind the Asset Browser context menu: create folder / material,
//! rename, duplicate, delete, reveal.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use kooch_core::asset_database::AssetDatabase;
use kooch_core::resource::Resources;
use kooch_render::material::Material;

mod main_scene;
mod owner;

pub(crate) use main_scene::main_scene_path;
use main_scene::set_main_scene;

use super::{EditorAction, NewFileKind};
use crate::systems::LastScannedProject;

/// Dispatches an Asset Browser file operation. Returns `true` if it
/// handled `action`, `false` otherwise (so the caller can keep matching).
pub(super) fn handle_asset_op(action: &EditorAction, resources: &mut Resources) -> bool {
    match action {
        EditorAction::CreateFolder { parent, name } => create_folder(parent, name),
        EditorAction::CreateMaterial {
            folder,
            name,
            shader,
        } => create_material(resources, folder, name, *shader),
        EditorAction::RenameAsset { path, new_name } => rename_asset(resources, path, new_name),
        EditorAction::RenameFolder { path, new_name } => rename_folder(resources, path, new_name),
        EditorAction::DuplicateAsset { path } => duplicate_asset(resources, path),
        EditorAction::DeleteAsset { path } => delete_asset(resources, path),
        EditorAction::DeleteFolder { path } => delete_folder(resources, path),
        EditorAction::RevealInFileManager { path } => reveal(path),
        EditorAction::SetMainScene { path } => set_main_scene(resources, path),
        EditorAction::OpenInIde { file } => open_in_ide(resources, file),
        EditorAction::OpenInputMap { path } => open_input_map(resources, path),
        EditorAction::OpenShaderGraph { path } => open_shader_graph(resources, path),
        EditorAction::SaveShaderGraph => save_shader_graph(resources),
        EditorAction::ShaderGraphFocused => {
            if let Some(open) = resources.get_mut::<crate::state::OpenShaderGraph>() {
                open.focus_requested = false;
            }
        }
        EditorAction::EditInputMap(edit) => edit_input_map(resources, edit),
        EditorAction::SaveInputMap => save_input_map(resources),
        EditorAction::InputMapFocused => {
            if let Some(open) = resources.get_mut::<crate::state::OpenInputMap>() {
                open.focus_requested = false;
            }
        }
        EditorAction::CreateFile { folder, name, kind } => {
            create_file(resources, folder, name, *kind)
        }
        EditorAction::InstallRequirements => {
            let report = resources.get::<crate::preflight::Report>().cloned();
            match report {
                Some(report) => {
                    if let Err(refusal) = crate::install::start(resources, &report) {
                        tracing::warn!("{refusal}");
                    }
                }
                None => tracing::error!("no preflight report to install from"),
            }
        }
        EditorAction::RegisterScripts => {
            // A click is a question, and all three outcomes used to look identical from the other
            // side of it: nothing happened. `Unchanged` is both the common answer and the confusing
            // one — most edits are to a body or a field, and this file names neither.
            if super::codegen::register_scripts(resources) == super::codegen::SyncOutcome::Unchanged
            {
                tracing::info!(
                    "registrations already name every component and system in src/ — a field, \
                     a body or a default is not in this file, and needs a rebuild rather than \
                     a rescan",
                );
            }
        }
        EditorAction::BuildProject(preset) => start_build(resources, *preset),
        EditorAction::CancelBuild => {
            if let Some(state) = resources.get_mut::<crate::build::BuildState>()
                && let Some(job) = state.job.as_mut()
            {
                job.cancel();
                tracing::info!("build cancelled");
            }
        }
        _ => return false,
    }
    true
}

// Baked-in fallbacks so file creation still works if the engine's
// `templates/` dir is missing at runtime; the on-disk copies are the
// editable source of truth.
const COMPONENT_TMPL: &str = include_str!("../../../../templates/component.rs.tmpl");
const SYSTEM_TMPL: &str = include_str!("../../../../templates/system.rs.tmpl");

/// Writes a new asset file and gives it an identity.
fn write_asset(resources: &mut Resources, file: &Path, text: &str, what: &str) {
    match std::fs::write(file, text) {
        Ok(()) => asset_created(resources, file, what),
        Err(e) => {
            tracing::error!(file = %file.display(), error = %e, "failed to write {what}")
        }
    }
}

/// The half of [`write_asset`] that is not the write.
pub(crate) fn new_block_asset(
    resources: &mut Resources,
    shape: kooch_blockmesh::Shape,
) -> Option<(PathBuf, kooch_core::Guid)> {
    let folder = resources
        .get::<crate::project_state::ProjectState>()?
        .active_project
        .as_ref()?
        .root_path
        .join("assets")
        .join("blocks");
    if let Err(error) = std::fs::create_dir_all(&folder) {
        tracing::error!(folder = %folder.display(), %error, "cannot create the blocks folder");
        return None;
    }

    let file = unique_target(
        &folder,
        OsStr::new(&format!(
            "{}.{}",
            shape.label(),
            kooch_blockmesh::BLOCK_MESH_EXTENSION
        )),
    );
    let mesh = kooch_blockmesh::BlockShape::from(shape).build();
    let text = match ron::ser::to_string_pretty(&mesh, ron::ser::PrettyConfig::default()) {
        Ok(text) => text,
        Err(error) => {
            tracing::error!(%error, shape = shape.label(), "cannot serialise a block shape");
            return None;
        }
    };

    let guid = write_asset_guid(
        resources,
        &file,
        &text,
        "block mesh",
        std::any::type_name::<kooch_blockmesh::BlockMesh>(),
    )?;
    Some((file, guid))
}

/// Writes an asset, mints its identity, and answers with the GUID.
pub(crate) fn write_asset_guid(
    resources: &mut Resources,
    file: &Path,
    text: &str,
    what: &str,
    asset_type: &str,
) -> Option<kooch_core::Guid> {
    use kooch_core::asset_database::{AssetDatabase, AssetEntry};
    use kooch_core::asset_meta::{AssetMeta, write_meta};

    if let Err(e) = std::fs::write(file, text) {
        tracing::error!(file = %file.display(), error = %e, "failed to write {what}");
        return None;
    }

    // 🔴 Typed here, not left to whatever loads it first.
    let mut meta = AssetMeta::new();
    meta.asset_type = Some(asset_type.to_owned());
    if let Err(e) = write_meta(file, &meta) {
        tracing::error!(file = %file.display(), error = %e, "failed to write {what} identity");
        return None;
    }
    if let Some(database) = resources.get_mut::<AssetDatabase>() {
        let mtime = std::fs::metadata(file)
            .and_then(|meta| meta.modified())
            .unwrap_or_else(|_| std::time::SystemTime::now());
        database.register(
            meta.guid,
            AssetEntry {
                path: file.to_path_buf(),
                mtime,
                type_name: Some(asset_type.to_owned()),
            },
        );
    }

    asset_created(resources, file, what);
    Some(meta.guid)
}

fn asset_created(resources: &mut Resources, file: &Path, what: &str) {
    tracing::info!(file = %file.display(), "{what} created");
    force_rescan(resources);
    crate::actions::handlers::asset_saved(resources, file);
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
            // The `.meta` is never copied — two files sharing a guid are two files claiming one
            // identity — but the copy still needs one of its own, and it is given here rather than
            // left to the rescan's eager import.
            duplicate_identity(resources, path, &dest);
            tracing::info!(from = %path.display(), to = %dest.display(), "asset duplicated");
            force_rescan(resources);
        }
        Err(e) => tracing::error!(from = %path.display(), error = %e, "duplicate failed"),
    }
}

/// Gives a freshly copied asset an identity of its own.
fn duplicate_identity(resources: &mut Resources, source: &Path, dest: &Path) {
    let Ok(source_meta) = kooch_core::asset_meta::read_meta(source) else {
        return;
    };
    let meta = match source_meta.asset_type {
        Some(asset_type) => kooch_core::asset_meta::AssetMeta::with_type(asset_type),
        None => kooch_core::asset_meta::AssetMeta::new(),
    };
    if let Err(e) = kooch_core::asset_meta::write_meta(dest, &meta) {
        tracing::error!(path = %dest.display(), error = %e, "copy has no asset identity");
        return;
    }
    // Registered now rather than at the next project change, for the same
    // reason a saved prefab is: the scan only runs when the active project
    // changes, so a file created mid-session is otherwise invisible.
    crate::actions::handlers::asset_saved(resources, dest);
}

fn delete_asset(resources: &mut Resources, path: &Path) {
    if owner::refuses(resources, path) {
        return;
    }
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
    // Before `remove_dir_all`, which is the call that makes a mistake
    // here unrecoverable.
    if owner::refuses(resources, path) {
        return;
    }
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
pub(crate) fn unique_target(dir: &Path, name: &OsStr) -> PathBuf {
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

/// Starts a build from the preset `guid` names (#758).
fn start_build(resources: &mut Resources, guid: kooch_core::Guid) {
    if resources
        .get::<crate::build::BuildState>()
        .is_some_and(crate::build::BuildState::busy)
    {
        tracing::warn!("a build is already running");
        return;
    }

    let Some(state) = resources.get::<crate::project_state::ProjectState>() else {
        return;
    };
    let Some(project) = state.active_project.as_ref() else {
        tracing::warn!("no project open to build");
        return;
    };
    let (root, engine_root) = (project.root_path.clone(), state.engine_root.clone());

    let Some(handle) = kooch_ecs::reflect::asset_registry::load_handle::<crate::build::BuildPreset>(
        resources, guid,
    ) else {
        tracing::error!(%guid, "that build preset could not be loaded");
        return;
    };
    let Some(preset) = resources
        .get::<kooch_core::assets::Assets<crate::build::BuildPreset>>()
        .and_then(|assets| assets.get(handle))
        .cloned()
    else {
        return;
    };

    // Generated on first use and kept: the editor has to be able to open
    // the pack it shipped yesterday.
    let key = match crate::build::project_key(&root) {
        Ok(key) => key,
        Err(e) => {
            tracing::error!(error = %e, "could not read this project's pack key");
            return;
        }
    };
    let crate_name = crate::cargo_args::crate_name(&root.join("Cargo.toml"));
    // The allowlist, taken from the loaders this binary has linked in.
    // Captured now because the job outlives this frame.
    let known: Vec<String> = resources
        .get::<kooch_core::asset_loader::AssetServer>()
        .map(|server| {
            server
                .known_extensions()
                .iter()
                .map(|(ext, _)| (*ext).to_owned())
                .collect()
        })
        .unwrap_or_default();

    match crate::build::BuildJob::start(
        &preset,
        guid,
        &root,
        engine_root.as_deref(),
        &crate_name,
        key,
        known,
    ) {
        Ok(job) => {
            tracing::info!(
                platforms = ?preset.targets(),
                "build started",
            );
            if let Some(state) = resources.get_mut::<crate::build::BuildState>() {
                state.log.clear();
                state.job = Some(job);
            }
        }
        // The one that matters: a missing target names the `rustup`
        // command that fixes it, rather than failing ten minutes in with
        // a linker error.
        Err(why) => tracing::error!("{why}"),
    }
}

mod create;
mod editors;

use super::ide;
use create::*;
use editors::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod duplicate_tests;

#[cfg(test)]
mod delete_tests;

#[cfg(test)]
mod settings_tests;

#[cfg(test)]
mod input_map_tests;

#[cfg(test)]
mod input_map_editing_tests;

//! File-system asset operations behind the Asset Browser context menu:
//! create folder / material, rename, duplicate, delete, reveal.
//!
//! Each mutates the project's `assets/` tree on disk and, where needed,
//! drops stale [`AssetDatabase`] bindings and forces a re-scan so the
//! browser + pickers reflect the change next frame. None are undoable.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use kooch_core::asset_database::AssetDatabase;
use kooch_core::resource::Resources;
use kooch_render::material::Material;

mod main_scene;

pub(crate) use main_scene::main_scene_path;
use main_scene::set_main_scene;

use super::{EditorAction, NewFileKind};
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
        EditorAction::SetMainScene { path } => set_main_scene(resources, path),
        EditorAction::OpenInIde { file } => open_in_ide(resources, file),
        EditorAction::OpenInputMap { path } => open_input_map(resources, path),
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
        EditorAction::RegisterScripts => {
            // The outcome matters to the poll, not to a menu click.
            let _ = super::codegen::register_scripts(resources);
        }
        EditorAction::AcknowledgeScriptSync => {
            if let Some(sync) = resources.get_mut::<crate::script_sync::ScriptSync>() {
                sync.acknowledge();
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

fn create_file(resources: &mut Resources, folder: &Path, name: &str, kind: NewFileKind) {
    let (tmpl_file, fallback, ext) = match kind {
        NewFileKind::RustComponent => ("component.rs.tmpl", COMPONENT_TMPL, "rs"),
        NewFileKind::RustSystem => ("system.rs.tmpl", SYSTEM_TMPL, "rs"),
        NewFileKind::InputAction => {
            // Bound to nothing: an action is a name and a control type
            // until someone says what triggers it, and guessing a key
            // would collide with whatever already uses it.
            let file = unique_target(
                folder,
                OsStr::new(&format!(
                    "{}.{}",
                    to_pascal_case(name),
                    kooch_input::actions::INPUT_ACTION_EXTENSION
                )),
            );
            let action = kooch_input::actions::Action::new(
                to_snake_case(name),
                kooch_input::actions::ControlType::Button,
            );
            match kooch_input::actions::save_action(&action, &file) {
                Ok(guid) => tracing::info!(file = %file.display(), %guid, "input action created"),
                Err(e) => {
                    tracing::error!(file = %file.display(), error = %e, "failed to write action")
                }
            }
            return;
        }
        NewFileKind::BuildPreset => {
            let file = unique_target(
                folder,
                OsStr::new(&format!("{name}.{}", crate::build::BUILD_PRESET_EXTENSION)),
            );
            let preset = crate::build::BuildPreset::default();
            match crate::build::preset::to_ron(&preset) {
                Ok(text) => write_asset(resources, &file, &text, "build preset"),
                Err(e) => tracing::error!(error = %e, "failed to serialise build preset"),
            }
            return;
        }
        NewFileKind::RenderSettings => {
            // Through the same save-and-register path a material takes,
            // not a bare write: `apply_render_settings_system` finds this
            // by *type*, so a file with no `.meta` is a file the renderer
            // never reads — authored, saved, and inert (#759).
            let file = unique_target(
                folder,
                OsStr::new(&format!(
                    "{name}.{}",
                    kooch_render::settings::RENDER_SETTINGS_EXTENSION
                )),
            );
            let settings = kooch_render::settings::RenderSettings::default();
            match kooch_render::settings::to_ron(&settings) {
                Ok(text) => write_asset(resources, &file, &text, "render settings"),
                Err(e) => tracing::error!(error = %e, "failed to serialise render settings"),
            }
            return;
        }
        NewFileKind::Scene => {
            let file = unique_target(
                folder,
                OsStr::new(&format!("{name}.{}", crate::project::SCENE_EXTENSION)),
            );
            let doc = kooch_ecs::SceneDocument {
                // A new scene gets its identity now, so references into it are
                // stable from the first save.
                id: kooch_core::Guid::new_v4(),
                name: name.to_owned(),
                version: "1.0".to_owned(),
                entities: Vec::new(),
            };
            match doc.save(&file) {
                Ok(()) => tracing::info!(file = %file.display(), "scene created"),
                Err(e) => {
                    tracing::error!(file = %file.display(), error = %e, "failed to write scene")
                }
            }
            return;
        }
    };

    // Prefer the engine's on-disk template (editable), fall back to the
    // baked-in copy.
    let template = engine_template(resources, tmpl_file).unwrap_or_else(|| fallback.to_owned());
    let content = template
        .replace("{{Name}}", &to_pascal_case(name))
        .replace("{{name}}", &to_snake_case(name));

    // Source files live outside `assets/`, so the fs-walked tree picks
    // them up next frame with no re-scan.
    let file = unique_target(
        folder,
        OsStr::new(&format!("{}.{ext}", to_snake_case(name))),
    );
    match std::fs::write(&file, content) {
        Ok(()) => tracing::info!(file = %file.display(), "file created"),
        Err(e) => tracing::error!(file = %file.display(), error = %e, "failed to create file"),
    }
}

/// Reads `templates/<file>` from the engine root, if resolvable.
fn engine_template(resources: &Resources, file: &str) -> Option<String> {
    let root = resources
        .get::<crate::project_state::ProjectState>()?
        .engine_root
        .clone()?;
    std::fs::read_to_string(root.join("templates").join(file)).ok()
}

/// Converts `name` to a `PascalCase` Rust type identifier.
fn to_pascal_case(name: &str) -> String {
    let mut out = String::new();
    let mut capitalize = true;
    for c in name.chars() {
        if c.is_alphanumeric() {
            if capitalize {
                out.extend(c.to_uppercase());
                capitalize = false;
            } else {
                out.push(c);
            }
        } else {
            capitalize = true;
        }
    }
    out
}

/// Converts `name` to a `snake_case` file / function identifier.
fn to_snake_case(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_owned()
}

/// Opens `file` in an external IDE, with the crate root that owns it as
/// the workspace folder.
///
/// Tried in order: the command configured in Settings, `$KOOCH_IDE`,
/// `codium` / `code` on the PATH, and finally **whatever the desktop
/// says opens a source file** — which is the one that works on a system
/// where the IDE is installed by Flatpak, Homebrew or an AppImage and is
/// therefore not on our PATH at all.
///
/// # Why `xdg-open` is no longer the fallback
///
/// It was, and it is worse than failing: `xdg-open <file>` opens the file
/// with **no workspace**, and `xdg-open <folder>` opens the **file
/// manager**, since that is a directory's default handler. Both look like
/// success, which is exactly why the missing workspace went unnoticed.
fn open_in_ide(resources: &Resources, file: &Path) {
    let root = workspace_for(resources, file);
    let root = root.as_deref().unwrap_or_else(|| {
        // Nothing claims it: open the folder it sits in, which is still
        // more useful than opening the file with no workspace at all.
        file.parent().unwrap_or(file)
    });
    let configured = resources
        .get::<crate::project_state::ProjectState>()
        .and_then(|ps| ps.editor_config.ide_command.clone())
        .or_else(|| std::env::var("KOOCH_IDE").ok());

    let launched = match configured
        .as_deref()
        .and_then(super::ide::IdeCommand::parse)
    {
        Some(command) => spawn_ide(&command, root, file),
        None => {
            let on_path = ["codium", "code"]
                .into_iter()
                .filter_map(super::ide::IdeCommand::parse)
                .any(|command| spawn_ide(&command, root, file));
            on_path
                || super::ide::from_desktop_defaults()
                    .is_some_and(|command| spawn_ide(&command, root, file))
        }
    };
    if !launched {
        // No `xdg-open` fallback: for a folder it opens the file manager,
        // and for a file it opens an editor with no project. Saying so is
        // more useful than doing something that looks like it worked.
        tracing::error!(
            path = %file.display(),
            "no IDE could be launched — set one in Settings (a full path works, \
             e.g. /home/you/.local/bin/codium)",
        );
    }
}

/// The crate root a file belongs to: the project's, or the engine's for
/// something under the read-only engine assets.
///
/// **The project root, not its `assets/` folder** — a workspace without
/// `Cargo.toml` or `src/` is not the project, and opening one is the bug
/// this function exists to prevent.
fn workspace_for(resources: &Resources, file: &Path) -> Option<PathBuf> {
    let state = resources.get::<crate::project_state::ProjectState>()?;
    if let Some(project) = state.active_project.as_ref()
        && file.starts_with(&project.root_path)
    {
        return Some(project.root_path.clone());
    }
    // Engine assets are read-only, but opening them next to the engine's
    // own source is what makes them worth looking at.
    state
        .engine_root
        .as_ref()
        .filter(|engine| file.starts_with(engine))
        .cloned()
}

/// Spawns `ide`, appending `<root>` and, where the IDE understands it,
/// `-g <file>`.
fn spawn_ide(ide: &super::ide::IdeCommand, root: &Path, file: &Path) -> bool {
    let program = &ide.program;
    let mut command = std::process::Command::new(program);
    command.args(&ide.args).arg(root);
    // `-g` means "go to this file". Skipped for a directory, which is
    // not somewhere to go, and for editors that do not know the flag —
    // they would treat it as a filename and create a file called `-g`.
    if file != root && file.is_file() && ide.understands_goto() {
        command.arg("-g").arg(file);
    }

    // The full invocation, because when this opens the wrong thing the
    // only useful question is what was actually run — and answering it
    // from the outside means guessing at three layers at once.
    tracing::info!(
        ide = program,
        root = %root.display(),
        file = %file.display(),
        command = ?command,
        "launching IDE",
    );

    match command.spawn() {
        Ok(_) => true,
        Err(error) => {
            // Warn, not debug: this is the line that explains an "IDE
            // could not be launched", and at debug nobody ever saw it.
            tracing::warn!(ide = %program, %error, "IDE did not launch");
            false
        }
    }
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
    write_asset(resources, &file, &text, "material");
}

/// Writes a new asset file and gives it an identity.
///
/// The two steps after the write are what separate an asset from a file
/// that merely exists, and both are easy to leave out — which is how a
/// capability gets built and stays unreachable, the pattern #744 was:
///
/// - **Re-scan**, so eager import writes a `.meta` with a fresh GUID and
///   registers it as a typed asset. The whole tree, because this is the
///   one case with no `.meta` to register *from* — the scan is what
///   creates it.
/// - **Tell the project**, because the re-scan is this process only, and
///   an asset the running project never heard of cannot be assigned to
///   anything over there.
fn write_asset(resources: &mut Resources, file: &Path, text: &str, what: &str) {
    match std::fs::write(file, text) {
        Ok(()) => {
            tracing::info!(file = %file.display(), "{what} created");
            force_rescan(resources);
            crate::actions::handlers::asset_saved(resources, file);
        }
        Err(e) => {
            tracing::error!(file = %file.display(), error = %e, "failed to write {what}")
        }
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
            // The `.meta` is never copied — two files sharing a guid are
            // two files claiming one identity — but the copy still needs
            // one of its own, and it is given here rather than left to the
            // rescan's eager import.
            //
            // Eager import decides by extension from a fixed list, so it
            // knew nothing about `.prefab`: a duplicated prefab got no
            // identity at all, which made it unselectable, uninspectable
            // and unspawnable. Every asset type added later would have
            // broken the same way. Deriving the copy's type from the
            // source's own sidecar works for all of them.
            duplicate_identity(resources, path, &dest);
            tracing::info!(from = %path.display(), to = %dest.display(), "asset duplicated");
            force_rescan(resources);
        }
        Err(e) => tracing::error!(from = %path.display(), error = %e, "duplicate failed"),
    }
}

/// Gives a freshly copied asset an identity of its own.
///
/// A fresh guid carrying the *source's* type: the copy is a distinct asset
/// holding the same kind of thing. Does nothing when the source had no
/// identity — a plain file copied in the browser is still a plain file.
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

/// Starts a build from the preset `guid` names (#758).
///
/// Reads the preset from the asset server rather than taking a copy: it
/// is edited in the Inspector, which writes the file, and building with
/// a stale copy is a build whose settings are not the ones on screen.
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
            tracing::info!(target = %preset.target_triple, "build started");
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod duplicate_tests;

#[cfg(test)]
mod settings_tests;

/// The map a new `.inputmap` starts with.
///
/// Not empty. Move and jump, on keyboard and pad, are what every game
/// binds first — and a file that already shows a composite, its parts and
/// two devices on one action teaches the shape better than a blank list
/// plus documentation would.
fn starter_input_map(name: &str) -> kooch_input::actions::ActionMap {
    use kooch_input::actions::{
        Action, ActionMap, Binding, Composite, ControlPath, ControlType, DEFAULT_DEADZONE_MAX,
        DEFAULT_DEADZONE_MIN, PartName, Processor, VectorMode,
    };
    use kooch_input::ids::{GamepadAxis, GamepadButton, KeyCode};

    ActionMap::new(name)
        .add(
            Action::new("move", ControlType::Vector2)
                .bind_all([
                    Binding::composite(Composite::Vector2 {
                        mode: VectorMode::DigitalNormalized,
                    }),
                    Binding::part(PartName::Up, ControlPath::Key(KeyCode::KeyW)),
                    Binding::part(PartName::Down, ControlPath::Key(KeyCode::KeyS)),
                    Binding::part(PartName::Left, ControlPath::Key(KeyCode::KeyA)),
                    Binding::part(PartName::Right, ControlPath::Key(KeyCode::KeyD)),
                ])
                .bind_all([
                    // Radial, not per-axis: a per-axis deadzone leaves a
                    // square hole a diagonal push slips through.
                    Binding::composite(Composite::Vector2 {
                        mode: VectorMode::Analog,
                    })
                    .with(Processor::StickDeadzone {
                        min: DEFAULT_DEADZONE_MIN,
                        max: DEFAULT_DEADZONE_MAX,
                    }),
                    Binding::part(PartName::Up, ControlPath::Axis(GamepadAxis::LeftStickY)),
                    Binding::part(PartName::Right, ControlPath::Axis(GamepadAxis::LeftStickX)),
                ]),
        )
        .add(
            Action::new("jump", ControlType::Button)
                .bind(Binding::to(ControlPath::Key(KeyCode::Space)))
                .bind(Binding::to(ControlPath::Button(GamepadButton::South))),
        )
}

/// Reads an `.inputmap` and hands it to the panel.
///
/// Read here rather than through the asset server on purpose: the panel
/// edits this copy, and a draw that re-read the loaded asset every frame
/// would make the edited map and the loaded map two values of the same
/// thing. Saving writes back and the server picks it up on reload.
fn open_input_map(resources: &mut Resources, path: &std::path::Path) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            tracing::error!(file = %path.display(), error = %e, "could not read input map");
            return;
        }
    };
    // A standalone action opens as a map of one, so the panel needs no
    // second code path for it.
    let single = path
        .extension()
        .is_some_and(|e| e == kooch_input::actions::INPUT_ACTION_EXTENSION);
    let parsed = if single {
        ron::from_str::<kooch_input::actions::Action>(&text).map(|mut action| {
            action.ensure_id("");
            let name = action.name.clone();
            kooch_input::actions::ActionMap::new(name).add(action)
        })
    } else {
        ron::from_str::<kooch_input::actions::ActionMap>(&text).map(|mut map| {
            map.assign_missing_ids();
            map
        })
    };

    match parsed {
        Ok(map) => {
            tracing::info!(
                file = %path.display(),
                actions = map.actions.len(),
                "input map opened",
            );
            resources.insert(crate::state::OpenInputMap {
                path: path.to_path_buf(),
                kind: match single {
                    true => crate::state::OpenInputKind::SingleAction,
                    false => crate::state::OpenInputKind::Map,
                },
                map,
                focus_requested: true,
                selected: None,
                dirty: false,
            });
        }
        // Named rather than swallowed: a map that will not parse is a
        // file someone has to fix, and an empty panel says nothing about
        // which file or why.
        Err(e) => tracing::error!(
            file = %path.display(),
            error = %e,
            "input map could not be parsed",
        ),
    }
}

/// The processor list a target points at, if it points at one.
fn processors_of(
    map: &mut kooch_input::actions::ActionMap,
    to: crate::panels::input_map::ProcessorTarget,
) -> Option<&mut Vec<kooch_input::actions::Processor>> {
    use crate::panels::input_map::ProcessorTarget;
    match to {
        ProcessorTarget::Action(index) => map.actions.get_mut(index).map(|a| &mut a.processors),
        ProcessorTarget::Binding(at) => map
            .actions
            .get_mut(at.action)
            .and_then(|action| action.bindings.get_mut(at.binding))
            .map(|binding| &mut binding.processors),
    }
}

/// Applies one edit to the open map, in memory.
///
/// Nothing here writes a file. That is `SaveInputMap`, and the split is
/// the same one a prefab has: the document is the thing being edited, the
/// file is where it is eventually put.

/// What an input-map edit is called in the history, and whether it
/// continues the one before it.
///
/// `None` for the ones that are not edits at all — selecting a binding,
/// arming a rebind, saving. A history entry for "I clicked on it" is a
/// Ctrl+Z that appears to do nothing, which is worse than one that
/// misses.
fn undoable_step(
    edit: &crate::panels::input_map::InputMapAction,
) -> Option<(&'static str, Option<crate::history::MergeKey>)> {
    use crate::history::MergeKey;
    use crate::panels::input_map::InputMapAction as Edit;

    match edit {
        Edit::Save | Edit::Select(_) | Edit::BeginRebind(_) | Edit::CancelRebind => None,
        // Typed and dragged: these arrive per keystroke and per frame, so
        // they carry a key and collapse into one step.
        Edit::RenameAction { action, .. } => {
            Some(("Rename Action", Some(MergeKey::of(("rename", action)))))
        }
        Edit::SetProcessor { to, index, .. } => Some((
            "Edit Processor",
            Some(MergeKey::of((format!("{to:?}"), index))),
        )),
        Edit::SetComposite { at, .. } => {
            Some(("Edit Composite", Some(MergeKey::of(format!("{at:?}")))))
        }
        // Discrete: one click, one step.
        Edit::SetControlType { .. } => Some(("Set Control Type", None)),
        Edit::Rebind { .. } => Some(("Rebind", None)),
        Edit::RemoveBinding(_) => Some(("Remove Binding", None)),
        Edit::AddBinding { .. } => Some(("Add Binding", None)),
        Edit::AddComposite { .. } => Some(("Add Composite", None)),
        Edit::AddProcessor { .. } => Some(("Add Processor", None)),
        Edit::RemoveProcessor { .. } => Some(("Remove Processor", None)),
        Edit::MoveProcessor { .. } => Some(("Move Processor", None)),
        _ => Some(("Edit Input Map", None)),
    }
}

fn edit_input_map(resources: &mut Resources, edit: &crate::panels::input_map::InputMapAction) {
    use crate::panels::input_map::InputMapAction as Edit;
    use kooch_input::actions::{Action, Binding, ControlPath, ControlType, Role};
    use kooch_input::ids::KeyCode;

    // Before the edit lands, while the map still holds what an undo
    // wants. Reading the path first because `record` needs the document
    // and the borrow below is exclusive.
    let path = resources
        .get::<crate::state::OpenInputMap>()
        .map(|open| open.path.clone());
    if let Some(path) = path
        && let Some((label, key)) = undoable_step(edit)
    {
        crate::history::documents::record(
            resources,
            &crate::history::Document::InputMap(path),
            label,
            key,
        );
    }

    let Some(open) = resources.get_mut::<crate::state::OpenInputMap>() else {
        return;
    };
    let map = &mut open.map;
    let changed = match edit {
        Edit::Select(selection) => {
            open.selected = Some(*selection);
            false
        }
        Edit::RenameAction { action, name } => match map.actions.get_mut(*action) {
            Some(target) if target.name != *name => {
                target.name = name.clone();
                true
            }
            _ => false,
        },
        Edit::SetControlType {
            action,
            control_type,
        } => match map.actions.get_mut(*action) {
            Some(target) if target.control_type != *control_type => {
                target.control_type = *control_type;
                true
            }
            _ => false,
        },
        Edit::AddAction => {
            // Named for what it is until renamed. An empty name would
            // resolve to nothing and read as a broken action.
            let name = unique_action_name(map, "new_action");
            map.actions.push(Action::new(name, ControlType::Button));
            // Selected on the way in, so the properties pane is already
            // showing its name field. Otherwise a new action is a row
            // somewhere in a list with no hint that it wants a name.
            open.selected = Some(crate::panels::input_map::Selection::Action(
                map.actions.len() - 1,
            ));
            true
        }
        Edit::RemoveAction { action } => {
            if *action < map.actions.len() {
                map.actions.remove(*action);
                true
            } else {
                false
            }
        }
        Edit::AddBinding { action } => match map.actions.get_mut(*action) {
            // Unbound rather than guessing: `Space` on every new binding
            // would silently collide with whatever already uses it.
            Some(target) => {
                target
                    .bindings
                    .push(Binding::to(ControlPath::Key(KeyCode::Space)));
                true
            }
            None => false,
        },
        // The head plus one unbound part per name it declares. Adding a
        // bare head would put a composite in the list that reads as
        // nothing and gives no clue which parts are missing.
        Edit::AddComposite { action, composite } => match map.actions.get_mut(*action) {
            Some(target) => {
                use kooch_input::actions::PartName;
                target.bindings.push(Binding::composite(*composite));
                let head = target.bindings.len() - 1;
                for name in PartName::of(*composite) {
                    target
                        .bindings
                        .push(Binding::part(*name, ControlPath::Key(KeyCode::Space)));
                }
                // Selected so its Mode lands in the properties pane —
                // the setting that decides whether a diagonal outruns a
                // straight line, and the one nobody goes looking for.
                open.selected = Some(crate::panels::input_map::Selection::Binding(
                    crate::panels::input_map::BindingAddress {
                        action: *action,
                        binding: head,
                    },
                ));
                true
            }
            None => false,
        },
        Edit::SetComposite { at, composite } => match map
            .actions
            .get_mut(at.action)
            .and_then(|target| target.bindings.get_mut(at.binding))
        {
            // Only a head has parameters; aimed at a part this is a
            // no-op rather than a binding turned into something else.
            Some(binding) => match &mut binding.role {
                Role::CompositeHead(current) if current != composite => {
                    *current = *composite;
                    true
                }
                _ => false,
            },
            None => false,
        },
        Edit::AddProcessor { to, processor } => match processors_of(map, *to) {
            // Appended, so a new one runs last. Prepending would silently
            // change what every existing processor sees.
            Some(list) => {
                list.push(*processor);
                true
            }
            None => false,
        },
        Edit::SetProcessor {
            to,
            index,
            processor,
        } => match processors_of(map, *to).and_then(|list| list.get_mut(*index)) {
            Some(current) if current != processor => {
                *current = *processor;
                true
            }
            _ => false,
        },
        Edit::RemoveProcessor { to, index } => match processors_of(map, *to) {
            Some(list) if *index < list.len() => {
                list.remove(*index);
                true
            }
            _ => false,
        },
        Edit::MoveProcessor { to, index, delta } => match processors_of(map, *to) {
            Some(list) => {
                let target = *index as i32 + delta;
                // Refused rather than saturated: a move off the end that
                // silently stayed put would still mark the file unsaved.
                if target < 0 || target as usize >= list.len() {
                    false
                } else {
                    list.swap(*index, target as usize);
                    true
                }
            }
            None => false,
        },
        // A composite head takes its parts with it. Left behind they are
        // rows `groups` skips — saved to the file, read by nothing.
        Edit::RemoveBinding(at) => match map.actions.get_mut(at.action) {
            Some(target) if at.binding < target.bindings.len() => {
                let range = kooch_input::actions::group_range(&target.bindings, at.binding);
                target.bindings.drain(range);
                true
            }
            _ => false,
        },
        Edit::Rebind { at, path } => match map
            .actions
            .get_mut(at.action)
            .and_then(|a| a.bindings.get_mut(at.binding))
        {
            Some(binding) => {
                // Only what it reads changes. Processors and the part it
                // plays in a composite are properties of the binding, not
                // of the control behind it.
                binding.role = match &binding.role {
                    Role::Part { name, .. } => Role::Part {
                        name: *name,
                        path: *path,
                    },
                    _ => Role::Whole(*path),
                };
                true
            }
            None => false,
        },
        // Rebind prompts are panel state, not document edits, and Save
        // is routed before it gets here.
        Edit::BeginRebind(_) | Edit::CancelRebind | Edit::Save => false,
    };

    if changed {
        open.dirty = true;
    }
}

/// A name no other action in `map` already has.
fn unique_action_name(map: &kooch_input::actions::ActionMap, base: &str) -> String {
    if map.resolve(base).is_none() {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|name| map.resolve(name).is_none())
        .unwrap_or_else(|| base.to_owned())
}

/// Writes the open map back to its file.
fn save_input_map(resources: &mut Resources) {
    let Some((path, map, kind)) = resources
        .get::<crate::state::OpenInputMap>()
        .map(|open| (open.path.clone(), open.map.clone(), open.kind))
    else {
        return;
    };
    // A standalone action is unwrapped from the map of one it was opened
    // into, so the file keeps the shape it had.
    // Through `save_action` rather than a bare write: a file that reached
    // disk without its `.meta` is one nothing can reference.
    let written = match map.actions.first() {
        Some(action) => kooch_input::actions::save_action(action, &path),
        None => Err("the action was deleted; nothing to save".to_owned()),
    };
    let _ = kind;
    match written {
        Ok(guid) => {
            tracing::info!(file = %path.display(), %guid, "input map saved");
            // The panel edits a copy; without this the bindings on disk and
            // the ones the running project answers to stay different until
            // it is relaunched.
            crate::actions::handlers::asset_saved(resources, &path);
            if let Some(open) = resources.get_mut::<crate::state::OpenInputMap>() {
                open.dirty = false;
            }
        }
        // Left dirty on purpose: the edits are still the only copy that
        // has them, and clearing the flag would claim they are safe.
        Err(e) => tracing::error!(file = %path.display(), error = %e, "failed to save input map"),
    }
}

#[cfg(test)]
mod input_map_tests;

#[cfg(test)]
mod input_map_editing_tests;

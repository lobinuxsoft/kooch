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
        EditorAction::OpenInIde { file } => open_in_ide(resources, file),
        EditorAction::OpenInputMap { path } => open_input_map(resources, path),
        EditorAction::InputMapFocused => {
            if let Some(open) = resources.get_mut::<crate::state::OpenInputMap>() {
                open.focus_requested = false;
            }
        }
        EditorAction::CreateFile { folder, name, kind } => {
            create_file(resources, folder, name, *kind)
        }
        EditorAction::RegisterScripts => super::codegen::register_scripts(resources),
        _ => return false,
    }
    true
}

// Baked-in fallbacks so file creation still works if the engine's
// `templates/` dir is missing at runtime; the on-disk copies are the
// editable source of truth.
const COMPONENT_TMPL: &str = include_str!("../../../../templates/component.rs.tmpl");
const SYSTEM_TMPL: &str = include_str!("../../../../templates/system.rs.tmpl");

fn create_file(resources: &Resources, folder: &Path, name: &str, kind: NewFileKind) {
    let (tmpl_file, fallback, ext) = match kind {
        NewFileKind::RustComponent => ("component.rs.tmpl", COMPONENT_TMPL, "rs"),
        NewFileKind::RustSystem => ("system.rs.tmpl", SYSTEM_TMPL, "rs"),
        NewFileKind::InputMap => {
            // A map with the two actions every game starts with, bound on
            // both devices. An empty file would be honest and useless:
            // the first thing anyone does is add exactly these, and a
            // starting point is also documentation of the shape.
            let file = unique_target(
                folder,
                OsStr::new(&format!(
                    "{}.{}",
                    to_pascal_case(name),
                    kooch_input::actions::INPUT_MAP_EXTENSION
                )),
            );
            let map = starter_input_map(&to_pascal_case(name));
            match kooch_input::actions::to_ron(&map)
                .map_err(|e| e.to_string())
                .and_then(|text| std::fs::write(&file, text).map_err(|e| e.to_string()))
            {
                Ok(()) => tracing::info!(file = %file.display(), "input map created"),
                Err(e) => {
                    tracing::error!(file = %file.display(), error = %e, "failed to write input map")
                }
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
    crate::actions::handlers::register_saved_asset(resources, dest);
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

#[cfg(test)]
mod tests {
    use super::{to_pascal_case, to_snake_case};

    #[test]
    fn pascal_case_from_various_inputs() {
        assert_eq!(to_pascal_case("NewComponent"), "NewComponent");
        assert_eq!(to_pascal_case("player health"), "PlayerHealth");
        assert_eq!(to_pascal_case("enemy_ai"), "EnemyAi");
    }

    #[test]
    fn snake_case_from_various_inputs() {
        assert_eq!(to_snake_case("NewSystem"), "new_system");
        assert_eq!(to_snake_case("PlayerHealth"), "player_health");
        assert_eq!(to_snake_case("enemy ai"), "enemy_ai");
    }
}

#[cfg(test)]
mod duplicate_tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A copy is a distinct asset, so it needs an id of its own — sharing
    /// the source's would make two files claim one identity and whichever
    /// registered last would win.
    #[test]
    fn a_copy_gets_a_fresh_id_carrying_the_source_type() {
        let dir = scratch("kooch_dup_identity");
        let source = dir.join("Enemy.prefab");
        let dest = dir.join("Enemy_1.prefab");
        std::fs::write(&source, "()").unwrap();
        std::fs::write(&dest, "()").unwrap();
        let original = kooch_core::asset_meta::AssetMeta::with_type("test::Thing");
        kooch_core::asset_meta::write_meta(&source, &original).unwrap();

        let mut resources = kooch_core::resource::Resources::new();
        duplicate_identity(&mut resources, &source, &dest);

        let copy = kooch_core::asset_meta::read_meta(&dest).expect("the copy has an identity");
        assert_ne!(copy.guid, original.guid, "the copy aliased the original");
        assert_eq!(
            copy.asset_type,
            Some("test::Thing".to_owned()),
            "the copy holds the same kind of thing as its source",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A plain file copied in the browser is still a plain file. Inventing
    /// an identity for it would list it in asset pickers as a typeless
    /// entry nothing can load.
    #[test]
    fn copying_something_that_is_not_an_asset_stays_not_an_asset() {
        let dir = scratch("kooch_dup_plain");
        let source = dir.join("notes.txt");
        let dest = dir.join("notes_1.txt");
        std::fs::write(&source, "hello").unwrap();
        std::fs::write(&dest, "hello").unwrap();

        let mut resources = kooch_core::resource::Resources::new();
        duplicate_identity(&mut resources, &source, &dest);

        assert!(kooch_core::asset_meta::read_meta(&dest).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- which folder the IDE opens as its workspace -------------------

    use crate::project_state::{ActiveProject, ProjectState};
    use kooch_core::resource::Resources;

    fn resources_with_roots(project: &str, engine: &str) -> Resources {
        let mut state = ProjectState::new();
        state.active_project = Some(ActiveProject {
            manifest: crate::project::ProjectManifest::new("test"),
            root_path: PathBuf::from(project),
        });
        state.engine_root = Some(PathBuf::from(engine));
        let mut resources = Resources::new();
        resources.insert(state);
        resources
    }

    /// The bug this replaced: all three call sites passed the asset
    /// browser's root, so the IDE opened `<project>/assets` — a workspace
    /// with no `Cargo.toml` and no `src/`.
    #[test]
    fn a_project_asset_opens_the_crate_root_not_the_assets_folder() {
        let resources = resources_with_roots("/proj", "/engine");

        let workspace = workspace_for(&resources, Path::new("/proj/assets/Player.prefab"));

        assert_eq!(
            workspace,
            Some(PathBuf::from("/proj")),
            "the workspace must be the crate root, or there is no source to edit"
        );
    }

    #[test]
    fn a_source_file_opens_the_same_root_as_an_asset() {
        let resources = resources_with_roots("/proj", "/engine");
        assert_eq!(
            workspace_for(&resources, Path::new("/proj/src/player.rs")),
            workspace_for(&resources, Path::new("/proj/assets/Player.prefab")),
        );
    }

    /// Engine assets are read-only, and opening them beside the engine's
    /// own source is what makes them worth looking at.
    #[test]
    fn an_engine_asset_opens_the_engine_root() {
        let resources = resources_with_roots("/proj", "/engine");
        assert_eq!(
            workspace_for(&resources, Path::new("/engine/assets/meshes/cube.glb")),
            Some(PathBuf::from("/engine")),
        );
    }

    #[test]
    fn a_file_under_neither_root_claims_no_workspace() {
        let resources = resources_with_roots("/proj", "/engine");
        assert_eq!(workspace_for(&resources, Path::new("/tmp/stray.ron")), None);
    }

    #[test]
    fn no_project_open_claims_no_workspace() {
        let mut resources = Resources::new();
        resources.insert(ProjectState::new());
        assert_eq!(
            workspace_for(&resources, Path::new("/proj/assets/a.ron")),
            None
        );
    }
}

/// The map a new `.inputmap` starts with.
///
/// Not empty. Move and jump, on keyboard and pad, are what every game
/// binds first — and a file that already shows a composite, its parts and
/// two devices on one action teaches the shape better than a blank list
/// plus documentation would.
fn starter_input_map(name: &str) -> kooch_input::actions::ActionMap {
    use kooch_input::actions::{
        Action, ActionMap, Binding, Composite, ControlPath, ControlType, DEFAULT_DEADZONE_MAX,
        DEFAULT_DEADZONE_MIN, PartName, Processor, Vector2Mode,
    };
    use kooch_input::ids::{GamepadAxis, GamepadButton, KeyCode};

    ActionMap::new(name)
        .add(
            Action::new("move", ControlType::Vector2)
                .bind_all([
                    Binding::composite(Composite::Vector2 {
                        mode: Vector2Mode::DigitalNormalized,
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
                        mode: Vector2Mode::Analog,
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
    match ron::from_str::<kooch_input::actions::ActionMap>(&text) {
        Ok(map) => {
            tracing::info!(
                file = %path.display(),
                actions = map.actions.len(),
                "input map opened",
            );
            resources.insert(crate::state::OpenInputMap {
                path: path.to_path_buf(),
                map,
                focus_requested: true,
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

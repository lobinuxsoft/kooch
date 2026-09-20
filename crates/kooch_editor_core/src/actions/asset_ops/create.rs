//! New files from the Asset Browser: scripts from templates, folders, materials.

use super::*;

pub(super) fn create_file(resources: &mut Resources, folder: &Path, name: &str, kind: NewFileKind) {
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
                Ok(guid) => {
                    tracing::debug!(file = %file.display(), %guid, "input action written");
                    asset_created(resources, &file, "input action");
                }
                Err(e) => {
                    tracing::error!(file = %file.display(), error = %e, "failed to write action")
                }
            }
            return;
        }
        NewFileKind::BlockMesh => {
            // A cube, because a block tool that starts from nothing has
            // nothing to drag. One metre: a step and a half for the
            // character, and a unit against the snap grid.
            let file = unique_target(
                folder,
                OsStr::new(&format!("{name}.{}", kooch_blockmesh::BLOCK_MESH_EXTENSION)),
            );
            let cube = kooch_blockmesh::BlockMesh::cuboid(glam::Vec3::splat(0.5));
            match ron::ser::to_string_pretty(&cube, ron::ser::PrettyConfig::default()) {
                // Through the identity-minting path rather than a plain
                // write: a block nothing registered cannot be named by
                // `Block.source`.
                Ok(text) => {
                    write_asset_guid(
                        resources,
                        &file,
                        &text,
                        "block mesh",
                        std::any::type_name::<kooch_blockmesh::BlockMesh>(),
                    );
                }
                Err(e) => tracing::error!(error = %e, "failed to serialise block mesh"),
            }
            return;
        }
        NewFileKind::Shader => {
            let file = unique_target(
                folder,
                OsStr::new(&format!(
                    "{name}.{}",
                    kooch_render::material::SHADER_EXTENSION
                )),
            );
            write_asset(
                resources,
                &file,
                kooch_render::material::NEW_SURFACE_SHADER,
                "shader",
            );
            return;
        }
        NewFileKind::ShaderGraph => {
            let file = unique_target(
                folder,
                OsStr::new(&format!(
                    "{name}.{}",
                    kooch_render::material::SHADER_EXTENSION
                )),
            );
            let graph = crate::shader_graph::starter();
            match crate::shader_graph::generate(&graph) {
                Ok(source) => write_asset(resources, &file, &source, "shader graph"),
                Err(reason) => tracing::error!("a new graph does not generate a shader: {reason}"),
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
        NewFileKind::Layers => {
            // Named after the project, like its settings: one table per project, found by type.
            let file = unique_target(
                folder,
                OsStr::new(&format!("{name}.{}", kooch_core::layers::LAYERS_EXTENSION)),
            );
            let names = kooch_core::layers::LayerNames::default();
            match ron::ser::to_string_pretty(&names, ron::ser::PrettyConfig::default()) {
                Ok(text) => write_asset(resources, &file, &text, "layer names"),
                Err(e) => tracing::error!(error = %e, "failed to serialise layer names"),
            }
            return;
        }
        NewFileKind::RenderSettings => {
            // Through the same save-and-register path a material takes, not a bare write:
            // `apply_render_settings_system` finds this by *type*, so a file with no `.meta` is a
            // file the renderer never reads — authored, saved, and inert (#759).
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
pub(super) fn engine_template(resources: &Resources, file: &str) -> Option<String> {
    let root = resources
        .get::<crate::project_state::ProjectState>()?
        .engine_root
        .clone()?;
    std::fs::read_to_string(root.join("templates").join(file)).ok()
}

/// Converts `name` to a `PascalCase` Rust type identifier.
pub(super) fn to_pascal_case(name: &str) -> String {
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
pub(super) fn to_snake_case(name: &str) -> String {
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

/// Opens `file` in an external IDE, with the crate root that owns it as the workspace folder.
pub(super) fn open_in_ide(resources: &Resources, file: &Path) {
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

/// The crate root a file belongs to: the project's, or the engine's for something under the
/// read-only engine assets.
pub(super) fn workspace_for(resources: &Resources, file: &Path) -> Option<PathBuf> {
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
pub(super) fn spawn_ide(ide: &super::ide::IdeCommand, root: &Path, file: &Path) -> bool {
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

pub(super) fn create_folder(parent: &Path, name: &str) {
    let dir = unique_target(parent, OsStr::new(name));
    match std::fs::create_dir_all(&dir) {
        // A fresh empty folder needs no re-scan — the tree walks the
        // filesystem, so it appears next frame on its own.
        Ok(()) => tracing::info!(dir = %dir.display(), "folder created"),
        Err(e) => tracing::error!(dir = %dir.display(), error = %e, "failed to create folder"),
    }
}

pub(super) fn create_material(
    resources: &mut Resources,
    folder: &Path,
    name: &str,
    shader: Option<kooch_core::Guid>,
) {
    let file = unique_target(
        folder,
        OsStr::new(&format!(
            "{name}.{}",
            kooch_render::material::MATERIAL_EXTENSION
        )),
    );
    let material = Material {
        shader,
        ..Material::default()
    };
    let text = match ron::ser::to_string_pretty(&material, ron::ser::PrettyConfig::default()) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to serialise new material");
            return;
        }
    };
    write_asset(resources, &file, &text, "material");
}

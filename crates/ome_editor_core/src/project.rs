//! Project manifest and file system operations.
//!
//! Handles `project.ome` manifests, project directory creation,
//! and persistent editor configuration (recent projects).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use ome_ecs::reflect::ReflectValue;
use ome_ecs::scene::{ComponentDescription, EntityDescription, SceneDocument};

/// Convention path of the default scene relative to the project root.
pub const DEFAULT_SCENE_REL_PATH: &str = "scenes/default.ome_scene";

// ---------------------------------------------------------------------------
// Project manifest (project.ome)
// ---------------------------------------------------------------------------

/// The project manifest stored in `project.ome` at the project root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub name: String,
    pub version: String,
    pub engine_version: String,
    pub main_scene: Option<String>,
    pub window: WindowSettings,
}

/// Window settings embedded in the project manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowSettings {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

impl ProjectManifest {
    /// Creates a new manifest with sensible defaults.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            version: "0.1.0".to_owned(),
            engine_version: env!("CARGO_PKG_VERSION").to_owned(),
            main_scene: None,
            window: WindowSettings {
                title: name.to_owned(),
                width: 1280,
                height: 720,
            },
        }
    }

    /// Saves the manifest to `project.ome` in the given directory.
    pub fn save(&self, project_root: &Path) -> Result<(), ProjectError> {
        let path = project_root.join("project.ome");
        let pretty = ron::ser::PrettyConfig::new()
            .struct_names(false)
            .enumerate_arrays(false);
        let contents = ron::ser::to_string_pretty(self, pretty)
            .map_err(|e| ProjectError::Serialize(e.to_string()))?;
        fs::write(&path, contents).map_err(ProjectError::Io)?;
        Ok(())
    }

    /// Loads a manifest from the `project.ome` file in the given directory.
    pub fn load(project_root: &Path) -> Result<Self, ProjectError> {
        let path = project_root.join("project.ome");
        if !path.exists() {
            return Err(ProjectError::NotAProject(project_root.to_path_buf()));
        }
        let contents = fs::read_to_string(&path).map_err(ProjectError::Io)?;
        let manifest: Self =
            ron::from_str(&contents).map_err(|e| ProjectError::Deserialize(e.to_string()))?;
        Ok(manifest)
    }
}

// ---------------------------------------------------------------------------
// Editor config (persistent, cross-session)
// ---------------------------------------------------------------------------

/// Persistent editor configuration, stored in the user's config directory.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EditorConfig {
    pub recent_projects: Vec<RecentProject>,
    /// External IDE command for "Open in IDE". A whitespace-separated
    /// program + args (e.g. `code` or `flatpak run com.vscodium.codium`);
    /// `<workspace> -g <file>` is appended. `None` = auto-detect.
    #[serde(default)]
    pub ide_command: Option<String>,
}

/// A recently opened project entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentProject {
    pub name: String,
    pub path: PathBuf,
}

impl EditorConfig {
    /// Returns the path to the editor config file.
    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("oh_my_engine").join("editor_config.ron"))
    }

    /// Loads the editor config from disk. Returns default if not found.
    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        let Ok(contents) = fs::read_to_string(&path) else {
            return Self::default();
        };
        ron::from_str(&contents).unwrap_or_default()
    }

    /// Saves the editor config to disk.
    pub fn save(&self) -> Result<(), ProjectError> {
        let path = Self::config_path().ok_or(ProjectError::NoConfigDir)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(ProjectError::Io)?;
        }
        let pretty = ron::ser::PrettyConfig::new()
            .struct_names(false)
            .enumerate_arrays(false);
        let contents = ron::ser::to_string_pretty(self, pretty)
            .map_err(|e| ProjectError::Serialize(e.to_string()))?;
        fs::write(&path, contents).map_err(ProjectError::Io)?;
        Ok(())
    }

    /// Adds a project to the recent list (or moves it to the top).
    pub fn add_recent(&mut self, name: &str, path: &Path) {
        self.recent_projects.retain(|r| r.path != path);
        self.recent_projects.insert(
            0,
            RecentProject {
                name: name.to_owned(),
                path: path.to_owned(),
            },
        );
        // Keep list reasonable.
        self.recent_projects.truncate(20);
    }

    /// Removes a project from the recent list by path.
    pub fn remove_recent(&mut self, path: &Path) {
        self.recent_projects.retain(|r| r.path != path);
    }
}

// ---------------------------------------------------------------------------
// Project creation
// ---------------------------------------------------------------------------

/// Standard project subdirectories.
const PROJECT_DIRS: &[&str] = &["scenes", "assets", "scripts", "src"];

/// Sanitizes a project name into a valid Rust crate name.
///
/// Lowercases, replaces spaces/hyphens with underscores, strips
/// non-alphanumeric/underscore characters.
pub fn sanitize_crate_name(name: &str) -> String {
    name.to_lowercase()
        .replace([' ', '-'], "_")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// Generates a `Cargo.toml` for a project crate.
fn generate_cargo_toml(name: &str, engine_root: &Path) -> String {
    let crate_name = sanitize_crate_name(name);
    // Cargo.toml requires forward slashes even on Windows.
    let engine_path = engine_root.display().to_string().replace('\\', "/");
    format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2024"

[workspace]

# Two artefacts from one crate. The `dylib` is what the standalone editor
# loads to learn this project's component types without compiling them;
# the `rlib` beside it is what the binary below links, so the game is an
# ordinary statically linked executable.
[lib]
crate-type = ["rlib", "dylib"]

[[bin]]
name = "{crate_name}"
path = "src/main.rs"

[dependencies]
# `editor` pulls in the embedded editor so `cargo run` opens the editor
# with this project's components; `cargo run -- --game` runs the game;
# `physics-debug-render` lets the host answer the editor's physics overlay
# — without it the solver walk is not compiled and the overlay draws
# nothing (#634).
# `dynamic` is what makes this project loadable by the standalone editor:
# without it `oh_my_engine::ome_plugin_api` is compiled out and the
# generated `lib.rs` does not build at all.
# `remote` lets `cargo run -- --remote` expose the ECS to the standalone
# editor over a local socket; `physics` gives you rigid bodies — without it a
# `RigidBody` is an inert component and nothing ever falls. `gravity` is
# the same story one level up: without it a `PointGravity` is authorable,
# mirrors to the editor, draws its gizmo, and pulls on nothing.
oh_my_engine = {{ path = "{engine_path}", features = ["editor", "physics", "gravity", "remote", "physics-debug-render", "dynamic"] }}
# Direct dep needed until `Reflect` proc-macro resolves through the facade.
ome_ecs = {{ path = "{engine_path}/crates/ome_ecs" }}
"#,
    )
}

/// Generates `src/lib.rs` — the project as a library the editor loads.
///
/// This is what lets the standalone editor know a project's component
/// types without compiling them: it loads the `dylib` this produces and
/// asks it to declare them. The binary links the same code as an `rlib`,
/// so the game is unaffected.
///
/// Editor-managed, like `registrations.rs`: regenerated when missing,
/// and its contents must stay in step with
/// [`crate::actions::codegen::render_registrations`].
pub(crate) fn generate_lib_rs(name: &str) -> String {
    let crate_name = sanitize_crate_name(name);
    format!(
        r##"//! AUTO-GENERATED by the Oh My Engine editor — do not edit by hand.
//!
//! Your project, as a library the editor can load. The `dylib` this
//! produces is what lets the standalone editor list your components
//! without compiling them.

// Editor-managed module: declares your components + systems.
pub mod registrations;

/// Declares this project's component types to the editor.
#[derive(Default)]
pub struct ProjectPlugin;

impl oh_my_engine::ome_plugin_api::OmePlugin for ProjectPlugin {{
    fn name(&self) -> &str {{
        "{crate_name}"
    }}

    fn build(&mut self, engine: &mut dyn oh_my_engine::ome_plugin_api::Engine) {{
        registrations::declare_components(engine);
    }}
}}

oh_my_engine::ome_plugin_api::export_plugin!(ProjectPlugin);
"##
    )
}

/// Generates a scaffold `src/main.rs` for a project crate.
///
/// Wires the editor-owned `registrations` module (see
/// [`INITIAL_REGISTRATIONS`]), which declares + registers the project's
/// components and systems. The editor regenerates that module — and this
/// `main.rs` if it goes missing — so the wiring here must stay in sync
/// with `crate::actions::codegen`.
pub(crate) fn generate_main_rs(name: &str) -> String {
    let crate_name = sanitize_crate_name(name);
    r##"use oh_my_engine::prelude::*;

// The project's own library — the same code the editor loads as a dylib.
// `registrations` is editor-managed: regenerated whenever you create or
// register scripts. Do not edit it by hand.
use PROJECT_CRATE::registrations;

fn main() {
    // `cargo run`            → the editor, with your components (authoring).
    // `cargo run -- --game`  → the game (what the editor's Play button runs).
    // `cargo run -- --remote`→ headless authoring host: your components +
    //                          the remote server, driven by the standalone
    //                          editor over a local socket. Gameplay starts
    //                          paused; the
    //                          editor's Play button starts it without a
    //                          rebuild, in the editor's own viewport.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--game") {
        // Game runtime: components + gameplay systems.
        let mut app = App::new();
        app.add_plugins(DefaultPlugins);
        app.add_plugin(registrations::ProjectRegistrations { run_systems: true });
        app.run();
    } else if args.iter().any(|a| a == "--remote") {
        // Remote authoring host: components register (so the editor's
        // Inspector sees them) and systems register paused — the editor
        // toggles `Playing` over the wire to run them in place.
        // Headless on purpose: the editor draws this world in its own
        // viewport, so a window here would show the same scene twice.
        let mut app = App::new();
        app.add_plugins(RemoteHostPlugins);
        app.add_plugin(registrations::ProjectRegistrations { run_systems: false });
        app.add_plugin(oh_my_engine::ome_remote::RemotePlugin::new());
        app.run();
    } else {
        // Editor embedded in the project: register components (for the
        // Inspector) but do NOT run gameplay systems.
        oh_my_engine::ome_editor_core::run_editor_with(registrations::ProjectRegistrations {
            run_systems: false,
        });
    }
}
"##
    .replace("PROJECT_CRATE", &crate_name)
}

/// Contents of a fresh, empty `src/registrations.rs` — valid on its own
/// so a new project compiles before any component/system exists. The
/// editor overwrites it with real registrations via
/// `crate::actions::codegen`.
pub(crate) const INITIAL_REGISTRATIONS: &str = "\
// AUTO-GENERATED by the Oh My Engine editor — do not edit by hand.
// Regenerated when you create or register components / systems.
#![allow(unused_imports, unused_variables, dead_code)]

use oh_my_engine::ome_ecs::component::ComponentRegistry;
use oh_my_engine::prelude::*;

/// Editor-managed plugin: registers project components + systems.
///
/// `run_systems` sets the starting value of the `Playing` gate: `true`
/// in the game build, `false` while editing. Systems are registered
/// either way and skipped per frame, so Play can flip it live.
pub struct ProjectRegistrations {
    pub run_systems: bool,
}

impl Plugin for ProjectRegistrations {
    fn build(&self, app: &mut App) {
        app.insert_resource(Playing(self.run_systems));
        app.add_system(Stage::Startup, register_components);
    }
}

fn register_components(resources: &mut Resources) {
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
}

/// Describes project components to an editor that loads this library.
///
/// Called from `lib.rs` when the editor loads the project's dylib.
pub fn declare_components(engine: &mut dyn oh_my_engine::ome_plugin_api::Engine) {
    use oh_my_engine::ome_ecs::component::plugin_bridge::declare_component;
}
";

/// Creates a new project directory with the standard structure and manifest.
///
/// `parent_dir` is the parent folder; a subdirectory named `name` will be
/// created inside it. `engine_root` is the path to the oh_my_engine repo
/// root, used to generate `Cargo.toml` dependency paths.
pub fn create_project(
    name: &str,
    parent_dir: &Path,
    engine_root: &Path,
) -> Result<PathBuf, ProjectError> {
    let project_root = parent_dir.join(name);
    if project_root.exists() {
        return Err(ProjectError::AlreadyExists(project_root));
    }
    fs::create_dir_all(&project_root).map_err(ProjectError::Io)?;
    for dir in PROJECT_DIRS {
        fs::create_dir_all(project_root.join(dir)).map_err(ProjectError::Io)?;
    }
    let mut manifest = ProjectManifest::new(name);
    manifest.main_scene = Some(DEFAULT_SCENE_REL_PATH.to_owned());
    manifest.save(&project_root)?;

    // Generate Cargo.toml.
    let cargo_toml = generate_cargo_toml(name, engine_root);
    fs::write(project_root.join("Cargo.toml"), cargo_toml).map_err(ProjectError::Io)?;

    // Generate src/main.rs scaffold + its editor-managed registrations.
    let main_rs = generate_main_rs(name);
    fs::write(project_root.join("src").join("main.rs"), main_rs).map_err(ProjectError::Io)?;
    let lib_rs = generate_lib_rs(name);
    fs::write(project_root.join("src").join("lib.rs"), lib_rs).map_err(ProjectError::Io)?;
    fs::write(
        project_root.join("src").join("registrations.rs"),
        INITIAL_REGISTRATIONS,
    )
    .map_err(ProjectError::Io)?;

    // Bootstrap the default scene file so the editor never opens empty.
    ensure_default_scene(&project_root)?;

    Ok(project_root)
}

/// Ensures `scenes/default.ome_scene` exists under `project_root`.
///
/// If the file is missing, writes a minimal starter scene with one Camera
/// entity (Transform + PerspectiveCamera + Name) and one Sky entity
/// (SkyRenderer + Name). All component fields are left empty so
/// `sync_scene_to_ecs` materializes them via `Reflect::reflect_default()`,
/// which means the starter tracks default changes without churn.
///
/// Returns the absolute path to the scene file.
pub fn ensure_default_scene(project_root: &Path) -> Result<PathBuf, ProjectError> {
    let scenes_dir = project_root.join("scenes");
    fs::create_dir_all(&scenes_dir).map_err(ProjectError::Io)?;

    let path = project_root.join(DEFAULT_SCENE_REL_PATH);
    if path.exists() {
        return Ok(path);
    }

    let doc = SceneDocument {
        // A new scene gets its identity now, so references into it are
        // stable from the first save.
        id: ome_core::Guid::new_v4(),
        name: "Default Scene".to_owned(),
        version: "0.1.0".to_owned(),
        entities: vec![
            EntityDescription {
                name: "Camera".to_owned(),
                parent_index: None,
                parent: None,
                components: vec![
                    ComponentDescription {
                        type_name: "ome_ecs::name::Name".to_owned(),
                        fields: vec![(
                            "value".to_owned(),
                            ReflectValue::String("Camera".to_owned()),
                        )],
                    },
                    ComponentDescription {
                        type_name: "ome_ecs::transform::Transform".to_owned(),
                        fields: vec![],
                    },
                    ComponentDescription {
                        type_name: "ome_ecs::perspective_camera::PerspectiveCamera".to_owned(),
                        fields: vec![],
                    },
                ],
            },
            EntityDescription {
                name: "Sky".to_owned(),
                parent_index: None,
                parent: None,
                components: vec![
                    ComponentDescription {
                        type_name: "ome_ecs::name::Name".to_owned(),
                        fields: vec![("value".to_owned(), ReflectValue::String("Sky".to_owned()))],
                    },
                    ComponentDescription {
                        type_name: "ome_ecs::sky_renderer::SkyRenderer".to_owned(),
                        fields: vec![],
                    },
                ],
            },
        ],
    };

    doc.save(&path)
        .map_err(|e| ProjectError::Serialize(e.to_string()))?;

    Ok(path)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during project operations.
#[derive(Debug)]
pub enum ProjectError {
    Io(std::io::Error),
    Serialize(String),
    Deserialize(String),
    NotAProject(PathBuf),
    AlreadyExists(PathBuf),
    NoConfigDir,
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Serialize(e) => write!(f, "failed to serialize: {e}"),
            Self::Deserialize(e) => write!(f, "failed to deserialize: {e}"),
            Self::NotAProject(p) => write!(f, "no project.ome found in {}", p.display()),
            Self::AlreadyExists(p) => write!(f, "directory already exists: {}", p.display()),
            Self::NoConfigDir => write!(f, "could not determine config directory"),
        }
    }
}

impl std::error::Error for ProjectError {}

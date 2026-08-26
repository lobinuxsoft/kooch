//! Project manifest and file system operations.
//!
//! Handles `project.kooch` manifests, project directory creation,
//! and persistent editor configuration (recent projects).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::scene::{ComponentDescription, EntityDescription, SceneDocument};

// The names live in `kooch_core` because the runtime's scene bootstrap needs
// them too, and it cannot depend on the editor to learn them. They were
// duplicated in both, which is how a rename changed one copy and left the
// runtime looking for a file the editor no longer wrote — see
// `kooch_core::scene_paths`.
pub use kooch_core::scene_paths::{
    DEFAULT_SCENE_REL_PATH, PREFAB_EXTENSION, PROJECT_MANIFEST_FILE, SCENE_EXTENSION,
};

// ---------------------------------------------------------------------------
// Project manifest (project.kooch)
// ---------------------------------------------------------------------------

/// The project manifest stored in `project.kooch` at the project root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub name: String,
    pub version: String,
    pub engine_version: String,
    pub main_scene: Option<String>,
    pub window: WindowSettings,
    /// Assets that ship even though no scene or prefab names them.
    ///
    /// 🔴 The packager ships what the game can REACH: scenes, prefabs,
    /// and everything those reference. A guid built in Rust — loaded by
    /// path at runtime, chosen from a table, assembled from a string —
    /// is not reachable by reading files, so it does not ship and the
    /// game misses it in silence.
    ///
    /// Every engine answers this with a declaration: Unity has
    /// `Resources/`, Godot has export filters. This is ours, and it is a
    /// list rather than a folder because the assets in question usually
    /// live in the ENGINE's tree, where a project cannot put a folder.
    #[serde(default)]
    pub build: BuildIncludes,
}

/// The `build` field of `project.kooch`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildIncludes {
    /// Project- or engine-relative paths, as they appear in the asset
    /// browser:
    ///
    /// ```ron
    /// build: (
    ///     include: ["assets/meshes/suzanne.glb"],
    /// ),
    /// ```
    ///
    /// Each is resolved to its guid and treated as a **root** of the
    /// same walk documents are, so declaring a material brings its
    /// textures without naming them too.
    #[serde(default)]
    pub include: Vec<String>,
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
            build: BuildIncludes::default(),
            window: WindowSettings {
                title: name.to_owned(),
                width: 1280,
                height: 720,
            },
        }
    }

    /// Saves the manifest to `project.kooch` in the given directory.
    pub fn save(&self, project_root: &Path) -> Result<(), ProjectError> {
        let path = project_root.join(PROJECT_MANIFEST_FILE);
        let pretty = ron::ser::PrettyConfig::new()
            .struct_names(false)
            .enumerate_arrays(false);
        let contents = ron::ser::to_string_pretty(self, pretty)
            .map_err(|e| ProjectError::Serialize(e.to_string()))?;
        fs::write(&path, contents).map_err(ProjectError::Io)?;
        Ok(())
    }

    /// Loads a manifest from the `project.kooch` file in the given directory.
    pub fn load(project_root: &Path) -> Result<Self, ProjectError> {
        let path = project_root.join(PROJECT_MANIFEST_FILE);
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
    /// Last address the Profiler panel connected to, e.g.
    /// `192.168.0.36:8585`.
    ///
    /// Remembered because it is a handheld's address on a home network:
    /// typed once, needed every session, and wrong in a way that looks
    /// like the profiler being broken.
    #[serde(default)]
    pub profiler_addr: Option<String>,
    /// Extra environment the Play button launches a project's game with,
    /// per project.
    ///
    /// Every knob this engine can be measured with is a `KOOCH_*`
    /// variable, because the frame they exist for is a game launched
    /// outside the editor. Play launches a game **from** the editor, and
    /// until this existed the only way to hand one a variable was to
    /// relaunch the editor with it set — the child inherits the parent's
    /// environment and nothing else.
    ///
    /// 🔴 Here rather than in `project.kooch` on purpose. A launch
    /// option is a measurement, and a measurement committed to a
    /// repository is a wrong configuration every collaborator then
    /// inherits — the same argument that keeps `KOOCH_SHADING_PAD` out
    /// of `.rendersettings`. The config directory cannot be committed by
    /// accident.
    ///
    /// Per project rather than one global string, because "it silently
    /// applied to the other project too" is exactly how a capture ends
    /// up measuring something nobody asked for.
    #[serde(default)]
    pub launch_env: Vec<ProjectLaunchEnv>,
}

/// One project's [`EditorConfig::launch_env`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectLaunchEnv {
    pub path: PathBuf,
    /// Whitespace-separated `KEY=VALUE`, as typed.
    ///
    /// Stored as the raw line rather than parsed pairs so the field
    /// shows back exactly what was written — including the part that did
    /// not parse, which is the part somebody needs to see to fix it.
    pub value: String,
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
        dirs::config_dir().map(|d| d.join("kooch").join("editor_config.ron"))
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

    /// The launch environment recorded for `project`, as typed.
    pub fn launch_env_for(&self, project: &Path) -> &str {
        self.launch_env
            .iter()
            .find(|e| e.path == project)
            .map(|e| e.value.as_str())
            .unwrap_or_default()
    }

    /// Records `value` for `project`. An empty line removes the entry
    /// rather than storing one, so clearing the field leaves no trace to
    /// wonder about later.
    pub fn set_launch_env(&mut self, project: &Path, value: String) {
        self.launch_env.retain(|e| e.path != project);
        if !value.trim().is_empty() {
            self.launch_env.push(ProjectLaunchEnv {
                path: project.to_path_buf(),
                value,
            });
        }
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
///
/// `scripts/` used to be here, from when a script meant a `.rhai` file on
/// disk. A script is a Rust component or system in `src/` now: codegen
/// scans `src/` and the Asset Browser's "Register scripts" reads Rust.
/// The scripting crate that loaded those files is gone. So the directory
/// was created, never read, and suggested a place to put code the engine
/// would never look at.
// `assets/scenes` rather than a top-level `scenes`: everything a game
// needs at runtime is one tree (#758).
const PROJECT_DIRS: &[&str] = &["assets/scenes", "assets", "src"];

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
///
/// `engine_path` is what goes in the `path` dependency. Normally it is
/// the relative [`engine_vendor::VENDOR_DIR`] — the engine copied into
/// the project — which is what makes the manifest identical on every
/// machine (#754). Developing the engine itself passes an absolute path
/// to the live clone instead; see [`create_project`].
/// [`generate_cargo_toml`] for tests that assert on the shape of the
/// manifest the editor writes.
///
/// Exists so the pieces the editor later *depends on* — the feature
/// names, the authoring binary — are checked against the real generator
/// rather than a copy of it in a test.
#[cfg(test)]
pub(crate) fn generate_cargo_toml_for_test(name: &str, engine_path: &str) -> String {
    generate_cargo_toml(name, engine_path)
}

fn generate_cargo_toml(name: &str, engine_path: &str) -> String {
    let crate_name = sanitize_crate_name(name);
    // Cargo.toml requires forward slashes even on Windows.
    let engine_path = engine_path.replace('\\', "/");
    format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2024"

[workspace]

# 🔴 A shipped game must not contain the editor (#558).
#
# `editor` is opt-in and the authoring binary below asks for it with
# `required-features`, so `cargo build --release` produces the game and
# cargo never puts the editor in its dependency graph at all. The
# guarantee belongs to the build, not to a `cfg` somebody has to get
# right — and `cargo tree` will show you.
#
# `physics` gives you rigid bodies: without it a `PhysicsBody` is an inert
# component and nothing ever falls. `gravity` is the same story one level
# up — a `PointGravity` is authorable, draws its gizmo, and pulls on
# nothing. `camera` is the third: a `VirtualCamera` that moves no camera.
[features]
default = ["game"]
game = ["kooch/physics", "kooch/gravity", "kooch/camera", "kooch/audio"]
# `dynamic` is what lets the standalone editor load this project's
# component types without compiling them; `remote` is how it drives them
# over a local socket; `physics-debug-render` compiles the solver walk the
# editor's overlay draws (#634). A game needs none of the three.
editor = [
    "game",
    "kooch/editor",
    "kooch/remote",
    "kooch/dynamic",
    "kooch/physics-debug-render",
]

# Two artefacts from one crate. The `dylib` is what the standalone editor
# loads to learn this project's component types without compiling them;
# the `rlib` beside it is what the binaries below link, so the game is an
# ordinary statically linked executable.
[lib]
crate-type = ["rlib", "dylib"]

# The game. No flags, no modes: double-clicking this plays.
[[bin]]
name = "{crate_name}"
path = "src/main.rs"

# Authoring — the embedded editor and the remote host. A separate target
# so a game build does not produce it, and `required-features` so a game
# build cannot.
[[bin]]
name = "{crate_name}_editor"
path = "src/editor.rs"
required-features = ["editor"]

[dependencies]
kooch = {{ path = "{engine_path}" }}
# Direct dep needed until `Reflect` proc-macro resolves through the facade.
kooch_ecs = {{ path = "{engine_path}/crates/kooch_ecs" }}
"#,
    )
}

/// Rewrites the manifest's engine dependency to point at `engine_dir`.
///
/// The path is absolute and `$HOME` differs per user, so a project that
/// moved between machines names a directory that is not there. That
/// line belongs to the editor — it owns the directory it names — so it
/// is corrected on open rather than left for cargo to fail on.
///
/// A no-op when it already matches, so opening a project does not
/// Moves a project onto an engine, **without opening or compiling it**.
///
/// 🔴 Two files record which engine a project uses, and writing one
/// without the other is what made the engine prompt return for ever
/// (#801): `Cargo.toml` carries the path cargo builds against, and
/// `project.kooch` carries `engine_version`, which is what decides
/// whether the prompt appears at all. They are written here together so
/// there is one place that can get it wrong.
///
/// Nothing is loaded and no build is started — which is the point.
/// Opening a project compiles its plugin first and discovers the version
/// mismatch second, throwing that compile away; settled here, the first
/// compile is already against the right engine (#800).
pub fn move_project_to_engine(
    project_root: &Path,
    engine_dir: &Path,
    version: &str,
) -> Result<(), ProjectError> {
    point_manifest_at_engine(project_root, engine_dir)?;
    let mut manifest = ProjectManifest::load(project_root)?;
    if manifest.engine_version != version {
        manifest.engine_version = version.to_owned();
        manifest.save(project_root)?;
    }
    Ok(())
}

/// The engine version a project records, without opening it.
///
/// `None` when there is no readable manifest — a directory that was
/// deleted or was never a project. The launcher shows those as missing
/// rather than guessing a version for them.
pub fn project_engine_version(project_root: &Path) -> Option<String> {
    ProjectManifest::load(project_root)
        .ok()
        .map(|m| m.engine_version)
}

/// rewrite its manifest for nothing.
pub fn point_manifest_at_engine(
    project_root: &Path,
    engine_dir: &Path,
) -> Result<bool, ProjectError> {
    let path = project_root.join("Cargo.toml");
    let text = fs::read_to_string(&path).map_err(ProjectError::Io)?;
    let engine = engine_dir.display().to_string().replace('\\', "/");

    let mut changed = false;
    let out: Vec<String> = text
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let rewritten = if trimmed.starts_with("kooch = {") {
                rewrite_path_value(line, &engine)
            } else if trimmed.starts_with("kooch_ecs = {") {
                rewrite_path_value(line, &format!("{engine}/crates/kooch_ecs"))
            } else {
                None
            };
            match rewritten {
                Some(new) if new != line => {
                    changed = true;
                    new
                }
                _ => line.to_owned(),
            }
        })
        .collect();

    if changed {
        fs::write(&path, out.join("\n") + "\n").map_err(ProjectError::Io)?;
    }
    Ok(changed)
}

/// Replaces the `path = "…"` value inside one dependency line.
fn rewrite_path_value(line: &str, value: &str) -> Option<String> {
    let key = line.find("path = \"")? + "path = \"".len();
    let end = key + line[key..].find('"')?;
    Some(format!("{}{value}{}", &line[..key], &line[end..]))
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
        r##"//! AUTO-GENERATED by the Kóoch editor — do not edit by hand.
//!
//! Your project, as a library the editor can load. The `dylib` this
//! produces is what lets the standalone editor list your components
//! without compiling them.

// Editor-managed module: declares your components + systems.
pub mod registrations;

// Everything below exists so the standalone editor can list your
// components without compiling them, and it is compiled out of a game
// build along with the rest of the authoring surface (#558) — a game
// loads no plugins, and `kooch::kooch_plugin_api` is not in its
// dependency graph to name.
#[cfg(feature = "editor")]
mod plugin {{
    use super::registrations;

    /// Declares this project's component types to the editor.
    #[derive(Default)]
    pub struct ProjectPlugin;

    impl kooch::kooch_plugin_api::KoochPlugin for ProjectPlugin {{
        fn name(&self) -> &str {{
            "{crate_name}"
        }}

        fn build(&mut self, engine: &mut dyn kooch::kooch_plugin_api::Engine) {{
            registrations::declare_components(engine);
        }}
    }}

    kooch::kooch_plugin_api::export_plugin!(ProjectPlugin);
}}

#[cfg(feature = "editor")]
pub use plugin::ProjectPlugin;
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
    r##"//! Your game.
//!
//! No flags and no modes: this is what a player runs, and it is the whole
//! of what a shipped build contains. Authoring lives in `src/editor.rs`,
//! behind the `editor` feature, so this binary cannot link it (#558).

use kooch::prelude::*;

// The project's own library — the same code the editor loads as a dylib.
// `registrations` is editor-managed: regenerated whenever you create or
// register scripts. Do not edit it by hand.
use PROJECT_CRATE::registrations;

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins);
    app.add_plugin(registrations::ProjectRegistrations { run_systems: true });
    app.run();
}
"##
    .replace("PROJECT_CRATE", &crate_name)
}

/// Generates `src/editor.rs` — authoring, in a target a game build does
/// not produce.
///
/// Split out of `main.rs` for #558. The old scaffold made the editor the
/// fall-through case of an argument match, so a shipped binary opened by
/// double-click started the *editor*; and the manifest asked for the
/// editor feature unconditionally, so the artefact carried the whole
/// authoring UI whether or not it could be reached.
///
/// A `cfg` would have expressed the intent. A separate target with
/// `required-features` enforces it: the game's build does not have
/// `kooch_editor_core` in its dependency graph at all.
pub(crate) fn generate_editor_rs(name: &str) -> String {
    let crate_name = sanitize_crate_name(name);
    r##"//! Authoring: the editor, and the host the standalone editor drives.
//!
//! Built only with `--features editor` (see `Cargo.toml`), so nothing
//! here can reach a shipped game.

use kooch::prelude::*;

use PROJECT_CRATE::registrations;

fn main() {
    // `cargo run --features editor --bin PROJECT_CRATE_editor`
    //     → the editor, with your components.
    // `… -- --remote`
    //     → headless authoring host: your components + the remote server,
    //       driven by the standalone editor over a local socket. Gameplay
    //       starts paused; the editor's Play button starts it without a
    //       rebuild, in the editor's own viewport.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--remote") {
        // Headless on purpose: the editor draws this world in its own
        // viewport, so a window here would show the same scene twice.
        let mut app = App::new();
        app.add_plugins(RemoteHostPlugins);
        app.add_plugin(registrations::ProjectRegistrations { run_systems: false });
        app.add_plugin(kooch::kooch_remote::RemotePlugin::new());
        app.run();
    } else {
        // Components register (so the Inspector sees them); gameplay
        // systems do not run until Play.
        kooch::kooch_editor_core::run_editor_with(registrations::ProjectRegistrations {
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

/// Creates a new project directory with the standard structure and manifest.
///
/// `parent_dir` is the parent folder; a subdirectory named `name` will be
/// created inside it. `engine_root` is the path to the kooch repo
/// root, used to generate `Cargo.toml` dependency paths.

/// What a new project tells git to leave alone.
///
/// `target/` is the whole point: a debug build of a project that links
/// this engine is gigabytes, and a repository that commits it is a
/// repository nobody can clone.
///
/// Two things are deliberately *not* here, because ignoring them breaks a
/// fresh clone:
///
/// - **`Cargo.lock`** — this crate builds a binary, and for a binary the
///   lock file is the record of what actually compiled. Libraries omit
///   it; games want the exact versions back.
/// - **`src/registrations.rs`** — editor-managed, but the build needs it:
///   `lib.rs` declares the module, so a clone without it does not compile
///   until the editor happens to regenerate it.
const PROJECT_GITIGNORE: &str = "\
# Rust build output. Gigabytes, and every byte of it regenerable.
/target

# 🔴 Editor-owned local state, and it holds this project's asset pack
# key. A repository that carries one has published it, and history keeps
# it published after the file is deleted. Keep a copy somewhere else:
# without it nobody can open the packs you already shipped.
/.kooch


# rustfmt leftovers.
**/*.rs.bk

# OS clutter.
.DS_Store
Thumbs.db
";

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

    // 🔴 The engine goes INSIDE the project (#754). Before this the
    // manifest carried an absolute path to whatever clone created the
    // project, so the project did not build on a second machine — and a
    // compiled editor, which has no clone next to it at all, could not
    // produce a buildable project.
    //
    // Developing the engine is the exception and has to stay working:
    // when the editor is running out of the engine's own source tree,
    // copying it would freeze the project against a snapshot and break
    // the daily loop of changing engine and game together. There the
    // manifest keeps pointing at the live clone.
    // 🔴 The engine is materialised ONCE per version on this machine and
    // shared by every project (#754) — not copied in here. Developing
    // the engine is the exception: the manifest points at the live clone
    // so a change to the engine reaches the game without a re-copy.
    let engine_path = if crate::engine_vendor::running_from_engine_build(engine_root) {
        engine_root.display().to_string()
    } else {
        let source = crate::engine_vendor::vendor_source(Some(engine_root));
        let version = crate::engine_vendor::editor_engine_version();
        match crate::engine_vendor::ensure_current(version, source.as_deref()) {
            Ok((_, Some(dir))) => dir.display().to_string(),
            // No engine to materialise: fall back to the root we were
            // handed. A manifest naming something is more useful than
            // one naming nothing, and the editor rewrites it on open.
            _ => engine_root.display().to_string(),
        }
    };

    // Generate Cargo.toml.
    let cargo_toml = generate_cargo_toml(name, &engine_path);
    fs::write(project_root.join("Cargo.toml"), cargo_toml).map_err(ProjectError::Io)?;

    // Generate src/main.rs scaffold + its editor-managed registrations.
    let main_rs = generate_main_rs(name);
    fs::write(project_root.join("src").join("main.rs"), main_rs).map_err(ProjectError::Io)?;
    let editor_rs = generate_editor_rs(name);
    fs::write(project_root.join("src").join("editor.rs"), editor_rs).map_err(ProjectError::Io)?;
    let lib_rs = generate_lib_rs(name);
    fs::write(project_root.join("src").join("lib.rs"), lib_rs).map_err(ProjectError::Io)?;
    fs::write(
        project_root.join("src").join("registrations.rs"),
        crate::actions::initial_registrations(),
    )
    .map_err(ProjectError::Io)?;

    // Written before anything large exists, so a project is never
    // briefly committable with its build output in it.
    fs::write(project_root.join(".gitignore"), PROJECT_GITIGNORE).map_err(ProjectError::Io)?;

    // Bootstrap the default scene file so the editor never opens empty.
    ensure_default_scene(&project_root)?;

    Ok(project_root)
}

/// Ensures `scenes/default.scene` exists under `project_root`.
///
/// If the file is missing, writes a minimal starter scene with one Camera
/// entity (Transform + PerspectiveCamera + Name) and one Sky entity
/// (SkyRenderer + Name). All component fields are left empty so
/// `sync_scene_to_ecs` materializes them via `Reflect::reflect_default()`,
/// which means the starter tracks default changes without churn.
///
/// Returns the absolute path to the scene file.
pub fn ensure_default_scene(project_root: &Path) -> Result<PathBuf, ProjectError> {
    // 🔴 Derived from the scene's own path, not spelled again. This said
    // `scenes` while the path below said `assets/scenes/default.scene`,
    // so it created one directory and wrote into another that did not
    // exist — "failed to ensure default scene: No such file or
    // directory", on every open, from a project that was fine.
    let path = project_root.join(DEFAULT_SCENE_REL_PATH);
    if let Some(scenes_dir) = path.parent() {
        fs::create_dir_all(scenes_dir).map_err(ProjectError::Io)?;
    }
    if path.exists() {
        return Ok(path);
    }

    let doc = SceneDocument {
        // A new scene gets its identity now, so references into it are
        // stable from the first save.
        id: kooch_core::Guid::new_v4(),
        name: "Default Scene".to_owned(),
        version: "0.1.0".to_owned(),
        entities: vec![
            EntityDescription {
                name: "Camera".to_owned(),
                parent_index: None,
                parent: None,
                components: vec![
                    ComponentDescription {
                        type_name: "kooch_ecs::name::Name".to_owned(),
                        fields: vec![(
                            "value".to_owned(),
                            ReflectValue::String("Camera".to_owned()),
                        )],
                    },
                    ComponentDescription {
                        type_name: "kooch_ecs::transform::Transform".to_owned(),
                        fields: vec![],
                    },
                    ComponentDescription {
                        type_name: "kooch_ecs::perspective_camera::PerspectiveCamera".to_owned(),
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
                        type_name: "kooch_ecs::name::Name".to_owned(),
                        fields: vec![("value".to_owned(), ReflectValue::String("Sky".to_owned()))],
                    },
                    ComponentDescription {
                        type_name: "kooch_ecs::sky_renderer::SkyRenderer".to_owned(),
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
            Self::NotAProject(p) => {
                write!(f, "no {PROJECT_MANIFEST_FILE} found in {}", p.display())
            }
            Self::AlreadyExists(p) => write!(f, "directory already exists: {}", p.display()),
            Self::NoConfigDir => write!(f, "could not determine config directory"),
        }
    }
}

impl std::error::Error for ProjectError {}

#[cfg(test)]
mod gitignore_tests;

#[cfg(test)]
mod vendoring_tests;

#[cfg(test)]
mod tests;

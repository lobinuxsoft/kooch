//! Project manifest and file system operations.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::scene::{ComponentDescription, EntityDescription, SceneDocument};

// The names live in `kooch_core` because the runtime's scene bootstrap needs them too, and it
// cannot depend on the editor to learn them.
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
    /// Last address the Profiler panel connected to, e.g. `192.168.0.36:8585`.
    #[serde(default)]
    pub profiler_addr: Option<String>,
    /// Extra environment the Play button launches a project's game with, per project.
    #[serde(default)]
    pub launch_env: Vec<ProjectLaunchEnv>,
}

/// One project's [`EditorConfig::launch_env`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectLaunchEnv {
    pub path: PathBuf,
    /// Whitespace-separated `KEY=VALUE`, as typed.
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
const PROJECT_DIRS: &[&str] = &["assets/scenes", "assets", "src"];

/// Sanitizes a project name into a valid Rust crate name.
pub fn sanitize_crate_name(name: &str) -> String {
    name.to_lowercase()
        .replace([' ', '-'], "_")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// Rewrites the manifest's engine dependency to point at `engine_dir`.
pub fn move_project_to_engine(
    project_root: &Path,
    engine_dir: &Path,
    version: &str,
) -> Result<(), ProjectError> {
    point_manifest_at_engine(project_root, engine_dir)?;
    // Moving engines is a full rebuild already, so this is when an older project gains it free.
    dev_profile::add_dev_profile(project_root)?;
    let mut manifest = ProjectManifest::load(project_root)?;
    if manifest.engine_version != version {
        manifest.engine_version = version.to_owned();
        manifest.save(project_root)?;
    }
    Ok(())
}

/// The engine version a project records, without opening it.
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

    // 🔴 The engine goes INSIDE the project (#754).
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
pub fn ensure_default_scene(project_root: &Path) -> Result<PathBuf, ProjectError> {
    // 🔴 Derived from the scene's own path, not spelled again.
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

mod dev_profile;

#[cfg(test)]
mod gitignore_tests;

#[cfg(test)]
mod vendoring_tests;

mod templates;

#[cfg(test)]
pub(crate) use templates::generate_cargo_toml_for_test;
use templates::*;
pub(crate) use templates::{generate_editor_rs, generate_lib_rs, generate_main_rs};

#[cfg(test)]
mod tests;

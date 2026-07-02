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

[dependencies]
oh_my_engine = {{ path = "{engine_path}" }}
# Direct dep needed until `Reflect` proc-macro resolves through the facade.
ome_ecs = {{ path = "{engine_path}/crates/ome_ecs" }}
"#,
    )
}

/// Generates a scaffold `src/main.rs` for a project crate.
///
/// The template is intentionally minimal: `DefaultPlugins` wires window,
/// ECS, the full render pipeline, and `SceneBootstrapPlugin` (which loads
/// the scene from `--scene <path>` or `scenes/default.ome_scene`).
///
/// `register_components` is left as a customization point for users who
/// add their own `#[derive(Reflect, Component)]` types.
fn generate_main_rs(_name: &str) -> String {
    r##"use oh_my_engine::ome_ecs::Reflect;
use oh_my_engine::ome_ecs::component::{Component, ComponentRegistry};
use oh_my_engine::prelude::*;

// -- Define your components here --
// #[derive(Default, Reflect)]
// struct Health { pub hp: u32, pub max_hp: u32 }
// impl Component for Health {}

/// Registers custom components for scene serialization.
/// Built-in components (Transform, Name) are registered by `EcsPlugin`.
fn register_components(resources: &mut Resources) {
    if let Some(_registry) = resources.get_mut::<ComponentRegistry>() {
        // registry.register_cpu_reflected::<Health>();
    }
}

fn main() {
    oh_my_engine::ome_core::init_tracing();
    let mut app = App::new();
    app.add_plugins(DefaultPlugins);
    app.add_system(Stage::Startup, register_components);
    app.run();
}
"##
    .to_owned()
}

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

    // Generate src/main.rs scaffold.
    let main_rs = generate_main_rs(name);
    fs::write(project_root.join("src").join("main.rs"), main_rs).map_err(ProjectError::Io)?;

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
        name: "Default Scene".to_owned(),
        version: "0.1.0".to_owned(),
        entities: vec![
            EntityDescription {
                name: "Camera".to_owned(),
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

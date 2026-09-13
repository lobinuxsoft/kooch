//! What scene and prefab files are called, in one place.

/// Extension of a scene file.
pub const SCENE_EXTENSION: &str = "scene";

/// Extension of a prefab file.
pub const PREFAB_EXTENSION: &str = "prefab";

/// Name of a project's manifest file, at the project root.
pub const PROJECT_MANIFEST_FILE: &str = "project.kooch";

/// Convention path of a project's default scene, relative to its root.
pub const DEFAULT_SCENE_REL_PATH: &str = "assets/scenes/default.scene";

/// Directory, under a project, that scenes live in.
pub const SCENES_DIR: &str = "assets/scenes";

/// Extensions the runtime reads **by path**, without going through a loader.
pub const READ_BY_PATH: [&str; 1] = [SCENE_EXTENSION];

/// The `main_scene` a manifest names, if it names one.
pub fn main_scene_of(manifest: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct BootFields {
        main_scene: Option<String>,
    }
    let fields: BootFields = ron::from_str(manifest).ok()?;
    fields.main_scene.filter(|s| !s.trim().is_empty())
}

/// The same path, tolerating the form that omits `assets/`.
pub fn normalise_main_scene(path: &str) -> String {
    let trimmed = path.trim_start_matches("./");
    if trimmed.starts_with("assets/") || !trimmed.starts_with("scenes/") {
        return trimmed.to_owned();
    }
    format!("assets/{trimmed}")
}

#[cfg(test)]
mod tests;

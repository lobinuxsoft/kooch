//! What scene and prefab files are called, in one place.
//!
//! # Why these live here rather than beside the code that uses them
//!
//! Three crates need to agree about these names: the editor writes them,
//! the runtime's scene bootstrap looks for them, and a project's manifest
//! records one. The editor cannot own them — the runtime would have to
//! depend on the editor to read them, which is backwards — so they sat
//! duplicated in both, and a rename changed one copy. The runtime then
//! went looking for a file the editor no longer wrote, and the
//! self-healing default-scene path wrote a fresh empty one over the gap.
//!
//! `kooch_core` is the one crate both already depend on.

/// Extension of a scene file.
pub const SCENE_EXTENSION: &str = "scene";

/// Extension of a prefab file.
///
/// # Why a second extension for one format
///
/// A prefab and a scene are the same document written by the same
/// serialiser — see
/// [`SceneDocument::from_ecs_subtree`](../../kooch_ecs/scene/document/struct.SceneDocument.html)
/// for why a second format would be a mistake. What differs is an
/// **invariant**: a prefab has exactly one root entity, a scene may have
/// any number.
///
/// With one extension there was nothing to tell them apart, so the editor
/// offered "instantiate" on every scene file and a four-root scene failed
/// at the point of instancing — a check the user had no way to anticipate.
/// The extension is what makes the invariant visible before the click, and
/// lets it be enforced when the file is *written* instead.
///
/// Unity draws the same line the same way: `.unity` and `.prefab` hold the
/// same YAML.
///
/// # Why not `kooch_`-prefixed
///
/// The prefix was namespacing against a collision that does not happen —
/// these files sit in a project's own `scenes/` and `assets/`, not in a
/// shared folder — and it made the *engine* the visible thing about a file
/// whose interesting property is its format.
pub const PREFAB_EXTENSION: &str = "prefab";

/// Name of a project's manifest file, at the project root.
///
/// # Why this one *is* named after the engine
///
/// The opposite of [`PREFAB_EXTENSION`]. A scene file's interesting
/// property is its format, so the engine's name has no business in it. A
/// manifest's interesting property is precisely *which engine owns this
/// directory* — it is the marker the launch screen looks for to decide a
/// folder is a project at all. Godot draws the same line with
/// `project.godot`.
///
/// # Why here and not beside the manifest
///
/// Same reason as the rest of this module: three places already agree
/// about this name — the manifest writes and reads it, the launch screen
/// tests for it to enable a row, and the delete guard in the launch
/// screen refuses to touch a directory without it. It was three string
/// literals until the engine was renamed and all three had to move.
pub const PROJECT_MANIFEST_FILE: &str = "project.kooch";

/// Convention path of a project's default scene, relative to its root.
///
/// Also the path the runtime falls back to relative to the working
/// directory when no `--scene` was passed; see the cwd caveat on
/// `SceneBootstrapPlugin`.
pub const DEFAULT_SCENE_REL_PATH: &str = "assets/scenes/default.scene";

/// Directory, under a project, that scenes live in.
///
/// 🔴 Under `assets/`, since #758. Everything a game needs at runtime is
/// in one tree, so packaging walks one place and "where does this file
/// go" has one answer. Projects made before this are moved when they
/// open.
pub const SCENES_DIR: &str = "assets/scenes";

/// Extensions the runtime reads **by path**, without going through a
/// loader.
///
/// 🔴 The packaging allowlist is derived from registered loaders, and a
/// scene has none: it is loaded by path, by name, before the asset system
/// is in play. Without this the scenes would be filtered out of a build
/// and the game would start empty — which is the failure the derived
/// allowlist otherwise prevents, arriving from the other side.
pub const READ_BY_PATH: [&str; 1] = [SCENE_EXTENSION];

/// The `main_scene` a manifest names, if it names one.
///
/// # Why the runtime parses the manifest at all
///
/// 🔴 Until #808 it did not, and `main_scene` was a field **nothing
/// read**. A shipped game opened [`DEFAULT_SCENE_REL_PATH`] whatever the
/// manifest said, so a project whose starting scene was not called
/// `default.scene` shipped a game that started somewhere else — or
/// started empty — with no error anywhere. The field looked like a
/// setting and behaved like a comment.
///
/// # Why a struct of one field rather than the editor's `ProjectManifest`
///
/// That type lives in `kooch_editor_core`, which a game does not link and
/// must not: the manifest carries authoring state the runtime has no
/// business knowing. Serde ignores the fields not named here, so this
/// keeps reading correctly as the manifest grows.
pub fn main_scene_of(manifest: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct BootFields {
        main_scene: Option<String>,
    }
    let fields: BootFields = ron::from_str(manifest).ok()?;
    fields.main_scene.filter(|s| !s.trim().is_empty())
}

/// The same path, tolerating the form that omits `assets/`.
///
/// ⚠️ `main_scene` is relative to the project **root** and therefore
/// starts with `assets/`, and projects exist on disk carrying
/// `scenes/x.scene` instead — `roll-a-ball` was one. Both look plausible
/// and only one resolves, so the short form is accepted and normalised
/// rather than silently resolving to a file that is not there.
///
/// Returns the path unchanged when it already names a directory that is
/// not `scenes/`, because that is somebody's deliberate layout and not
/// this function's business.
pub fn normalise_main_scene(path: &str) -> String {
    let trimmed = path.trim_start_matches("./");
    if trimmed.starts_with("assets/") || !trimmed.starts_with("scenes/") {
        return trimmed.to_owned();
    }
    format!("assets/{trimmed}")
}

#[cfg(test)]
mod tests;

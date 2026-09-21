//! Scene save / load wrappers — lift `SceneManager` out of `Resources`
//! during the operation to avoid overlapping borrows with
//! `sync_scene_to_ecs`.

use std::path::{Path, PathBuf};

use kooch_core::resource::Resources;

/// Loads a scene through `SceneManager`, lifting it out of `Resources`
/// while the load runs (avoids overlapping borrows with `sync_scene_to_ecs`).
pub(crate) fn load_scene(
    resources: &mut Resources,
    path: &Path,
) -> Result<(), kooch_ecs::SceneError> {
    let mut sm = resources
        .remove::<kooch_ecs::SceneManager>()
        .unwrap_or_default();
    let result = sm.load(path, resources);
    resources.insert(sm);
    result
}

/// Loads a scene beside the ones already open, returning its identity.
pub(super) fn open_scene_additive(
    resources: &mut Resources,
    path: &Path,
) -> Result<kooch_core::Guid, kooch_ecs::SceneError> {
    let mut sm = resources
        .remove::<kooch_ecs::SceneManager>()
        .unwrap_or_default();
    let result = sm.open_additive(path, resources);
    resources.insert(sm);
    result
}

/// Closes one scene. Returns `false` if it was not open.
pub(super) fn close_scene(resources: &mut Resources, id: kooch_core::Guid) -> bool {
    let mut sm = resources
        .remove::<kooch_ecs::SceneManager>()
        .unwrap_or_default();
    let closed = sm.close(id, resources);
    resources.insert(sm);
    closed
}

/// Saves the current ECS state to `path` via `SceneManager`, adopting it
/// as the new current scene.
pub(super) fn save_scene_as(
    resources: &mut Resources,
    path: PathBuf,
) -> Result<(), kooch_ecs::SceneError> {
    let mut sm = resources
        .remove::<kooch_ecs::SceneManager>()
        .unwrap_or_default();
    let result = sm.save_as(path, resources);
    resources.insert(sm);
    result
}

/// Saves one open scene to `path` via `SceneManager`, adopting it.
pub(crate) fn save_open_scene_as(
    resources: &mut Resources,
    id: kooch_core::Guid,
    path: PathBuf,
) -> Result<(), kooch_ecs::SceneError> {
    let mut sm = resources
        .remove::<kooch_ecs::SceneManager>()
        .unwrap_or_default();
    let result = sm.save_scene_as(id, path, resources);
    resources.insert(sm);
    result
}

/// Throws away one scene's edits and reads it back from its file.
pub(super) fn revert_scene(
    resources: &mut Resources,
    id: kooch_core::Guid,
) -> Result<(), kooch_ecs::SceneError> {
    let mut sm = resources
        .remove::<kooch_ecs::SceneManager>()
        .unwrap_or_default();
    let result = sm.revert(id, resources);
    resources.insert(sm);
    result
}

/// Where an open scene came from, or `None` for one never saved.
pub(super) fn scene_path(resources: &Resources, id: kooch_core::Guid) -> Option<PathBuf> {
    resources
        .get::<kooch_ecs::SceneManager>()?
        .scene(id)?
        .path
        .clone()
}

/// The dialog's answer, saying so when there was none: a save that does nothing in silence reads
/// as a save that is broken.
pub(crate) fn picked(path: Option<PathBuf>) -> Option<PathBuf> {
    if path.is_none() {
        tracing::info!("no file picked; nothing saved");
    }
    path
}

/// Builds the scene file dialog, opened beside `near` when given, else in the project's scenes.
pub(crate) fn scene_dialog(resources: &Resources, near: Option<&Path>) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new().add_filter("Scene", &[crate::project::SCENE_EXTENSION]);
    let root = resources
        .get::<crate::project_state::ProjectState>()
        .and_then(|ps| ps.active_project.as_ref().map(|p| p.root_path.clone()));
    if let Some(dir) = dialog_start(near, root.as_deref()) {
        dialog = dialog.set_directory(dir);
    }
    dialog
}

/// The first folder that exists: the scene's own, then `assets/scenes`, `assets`, the root. A
/// missing one is not an error to the portal — it silently opens somewhere else.
pub(crate) fn dialog_start(near: Option<&Path>, root: Option<&Path>) -> Option<PathBuf> {
    let own = near.and_then(Path::parent).map(Path::to_path_buf);
    let project = root.into_iter().flat_map(|root| {
        [
            root.join("assets/scenes"),
            root.join("assets"),
            root.to_path_buf(),
        ]
    });
    own.into_iter().chain(project).find(|dir| dir.is_dir())
}

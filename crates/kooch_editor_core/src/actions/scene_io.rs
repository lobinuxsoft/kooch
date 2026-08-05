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

/// Builds the scene file dialog, rooted at the active project's
/// `scenes/` folder when there is one.
///
/// Shared by the local handlers and the remote sink so both modes offer
/// the same picker — the two processes see the same filesystem, so the
/// path the user picks is meaningful on either side of the wire.
pub(crate) fn scene_dialog(resources: &Resources) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new().add_filter("Scene", &[crate::project::SCENE_EXTENSION]);
    if let Some(dir) = resources
        .get::<crate::project_state::ProjectState>()
        .and_then(|ps| {
            ps.active_project
                .as_ref()
                .map(|p| p.root_path.join("scenes"))
        })
    {
        dialog = dialog.set_directory(dir);
    }
    dialog
}

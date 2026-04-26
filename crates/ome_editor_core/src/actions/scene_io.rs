//! Scene save / load wrappers — lift `SceneManager` out of `Resources`
//! during the operation to avoid overlapping borrows with
//! `sync_scene_to_ecs`.

use std::path::{Path, PathBuf};

use ome_core::resource::Resources;

/// Loads a scene through `SceneManager`, lifting it out of `Resources`
/// while the load runs (avoids overlapping borrows with `sync_scene_to_ecs`).
pub(super) fn load_scene(
    resources: &mut Resources,
    path: &Path,
) -> Result<(), ome_ecs::SceneError> {
    let mut sm = resources
        .remove::<ome_ecs::SceneManager>()
        .unwrap_or_default();
    let result = sm.load(path, resources);
    resources.insert(sm);
    result
}

/// Saves the current ECS state to `path` via `SceneManager`, adopting it
/// as the new current scene.
pub(super) fn save_scene_as(
    resources: &mut Resources,
    path: PathBuf,
) -> Result<(), ome_ecs::SceneError> {
    let mut sm = resources
        .remove::<ome_ecs::SceneManager>()
        .unwrap_or_default();
    let result = sm.save_as(path, resources);
    resources.insert(sm);
    result
}

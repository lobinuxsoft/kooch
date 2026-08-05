//! Scene I/O: save, open, open additive, close, and switching the active
//! one.

use kooch_core::resource::Resources;

use crate::undo::UndoStack;

use crate::actions::scene_io::{
    close_scene, load_scene, open_scene_additive, save_scene_as, scene_dialog,
};

pub(super) fn handle_save_scene(resources: &mut Resources) {
    let Some(path) = scene_dialog(resources).save_file() else {
        return;
    };
    match save_scene_as(resources, path.clone()) {
        Ok(()) => tracing::info!("scene saved to {}", path.display()),
        Err(e) => tracing::error!("failed to save scene: {e}"),
    }
}

pub(super) fn handle_open_scene(resources: &mut Resources, undo_stack: &mut UndoStack) {
    let Some(path) = scene_dialog(resources).pick_file() else {
        return;
    };
    match load_scene(resources, &path) {
        Ok(()) => {
            tracing::info!("scene loaded from {}", path.display());
            undo_stack.clear();
        }
        Err(e) => tracing::error!("failed to load scene: {e}"),
    }
}

/// Opens a scene beside the ones already loaded.
///
/// The undo stack is left alone: nothing that was already open changed,
/// so the history of edits to those scenes is still valid. A replacing
/// load clears it because the entities those edits name are gone.
pub(super) fn handle_open_scene_additive(resources: &mut Resources) {
    // The menu greys this out while mirroring a project, but the action
    // can reach here by other routes (a shortcut, a replayed action), and
    // the failure it guards against is silent: entities that exist only
    // on this side, invisible in the game, whose every edit is dropped.
    if resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|state| state.is_connected())
    {
        tracing::warn!(
            "additive scene loading is unavailable while a project is open; \
             the world shown here mirrors the project",
        );
        return;
    }

    let Some(path) = scene_dialog(resources).pick_file() else {
        return;
    };
    match open_scene_additive(resources, &path) {
        Ok(id) => tracing::info!("scene {id} loaded additively from {}", path.display()),
        Err(e) => tracing::error!("failed to load scene: {e}"),
    }
}

/// Closes one scene, despawning only its entities.
///
/// The undo stack is cleared: entries naming entities that just went away
/// would resurrect nothing on undo.
pub(super) fn handle_close_scene(resources: &mut Resources, id: kooch_core::Guid) {
    if close_scene(resources, id) {
        tracing::info!("scene {id} closed");
    } else {
        tracing::warn!("asked to close scene {id}, which is not open");
    }
}

pub(super) fn handle_set_active_scene(resources: &mut Resources, id: kooch_core::Guid) {
    if let Some(sm) = resources.get_mut::<kooch_ecs::SceneManager>()
        && !sm.set_active(id)
    {
        tracing::warn!("asked to activate scene {id}, which is not open");
    }
}

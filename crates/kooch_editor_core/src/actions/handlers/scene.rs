//! Scene I/O: save, open, open additive, close, and switching the active
//! one.

use kooch_core::resource::Resources;

use crate::undo::UndoStack;

use crate::actions::scene_io::{
    close_scene, load_scene, open_scene_additive, revert_scene, save_open_scene_as, save_scene_as,
    scene_dialog, scene_path,
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

/// Saves one named open scene.
///
/// `as_new` asks for a path; otherwise the scene is written back to the
/// file it came from, and only a scene that has never been saved falls
/// through to the dialog — there is nothing else it could be written to.
pub(super) fn handle_save_open_scene(
    resources: &mut Resources,
    id: kooch_core::Guid,
    as_new: bool,
) {
    let existing = (!as_new).then(|| scene_path(resources, id)).flatten();
    let path = match existing {
        Some(path) => path,
        None => match scene_dialog(resources).save_file() {
            Some(path) => path,
            None => return,
        },
    };
    match save_open_scene_as(resources, id, path.clone()) {
        Ok(()) => tracing::info!("scene {id} saved to {}", path.display()),
        Err(e) => tracing::error!("failed to save scene {id}: {e}"),
    }
}

/// Throws away one scene's edits and reads it back from its file.
///
/// The undo stack is cleared: its entries name entities that were just
/// despawned, so undoing one would resurrect nothing.
pub(super) fn handle_revert_open_scene(
    resources: &mut Resources,
    id: kooch_core::Guid,
    undo_stack: &mut UndoStack,
) {
    match revert_scene(resources, id) {
        Ok(()) => {
            tracing::info!("scene {id} reverted to its file");
            undo_stack.clear();
        }
        Err(e) => tracing::error!("failed to revert scene {id}: {e}"),
    }
}

/// Replaces the world with a scene.
///
/// `named` is the file when the caller already had one — an Assets panel
/// row is a path, and raising a dialog for the file just clicked is the
/// same fault as having no way to name it.
pub(super) fn handle_open_scene(
    resources: &mut Resources,
    undo_stack: &mut UndoStack,
    named: Option<std::path::PathBuf>,
) {
    let path = match named {
        Some(path) => path,
        None => match scene_dialog(resources).pick_file() {
            Some(path) => path,
            None => return,
        },
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
pub(super) fn handle_open_scene_additive(
    resources: &mut Resources,
    named: Option<std::path::PathBuf>,
) {
    // 🔴 The connected case is no longer refused here — it is ROUTED, to
    // `Method::LoadSceneAdditive`, so the scene arrives where the world
    // lives. Reaching this function while connected now means the route
    // was not taken, which is a bug rather than a mode.
    if resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|state| state.is_connected())
    {
        tracing::warn!(
            "additive load fell through to the local path while a project is open; \
             the scene would exist only on this side",
        );
        return;
    }

    let path = match named {
        Some(path) => path,
        None => match scene_dialog(resources).pick_file() {
            Some(path) => path,
            None => return,
        },
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

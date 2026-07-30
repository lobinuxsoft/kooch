//! Handlers for non-ECS [`EditorAction`] variants.
//!
//! Split by the part of the editor each one moves — assets, scenes,
//! play, the project, the remote session, settings — with the dispatcher
//! here. `remote` is deliberately its own file: it is the half that goes
//! away when the remote protocol is demolished, and keeping it separate
//! makes that a deletion rather than a surgery.

mod assets;
mod play;
mod project;
mod remote;
mod scene;
mod settings;

use ome_core::resource::Resources;

use crate::undo::UndoStack;

use crate::actions::EditorAction;

/// Dispatches a non-ECS, non-undo action to the appropriate handler.
/// ECS actions (`Spawn`, `Despawn`, `SetField`, `AddComponent`,
/// `RemoveComponent`) plus `Undo` / `Redo` are handled by the caller —
/// this function is a no-op for them.
pub(super) fn apply_non_ecs_action(
    action: &EditorAction,
    resources: &mut Resources,
    undo_stack: &mut UndoStack,
) {
    // Asset Browser file operations (create / rename / delete / …) live
    // in their own module; delegate first, fall through to the rest.
    if crate::actions::asset_ops::handle_asset_op(action, resources) {
        return;
    }
    match action {
        EditorAction::SaveScene => handle_save_scene(resources),
        EditorAction::OpenScene => handle_open_scene(resources, undo_stack),
        EditorAction::OpenSceneAdditive => handle_open_scene_additive(resources),
        EditorAction::CloseScene(id) => handle_close_scene(resources, *id),
        EditorAction::SetActiveScene(id) => handle_set_active_scene(resources, *id),
        EditorAction::Play => handle_play(resources),
        EditorAction::Stop => handle_stop(resources),
        EditorAction::OpenProject(path) => handle_open_project(resources, path),
        EditorAction::RebuildRemote => handle_rebuild_remote(resources),
        EditorAction::CreateProject { name, parent_path } => {
            handle_create_project(resources, name, parent_path);
        }
        EditorAction::CloseProject => handle_close_project(resources, undo_stack),
        EditorAction::Reparent { entity, new_parent } => {
            handle_reparent(resources, *entity, *new_parent);
        }
        EditorAction::RemoveRecent(path) => handle_remove_recent(resources, path),
        EditorAction::CleanProject => handle_clean_project(resources),
        EditorAction::LaunchProject(path) => handle_launch_project(resources, path),
        EditorAction::CancelLaunch => handle_cancel_launch(resources),
        EditorAction::SetPowerProfile(profile) => handle_set_power_profile(resources, *profile),
        EditorAction::SetIdeCommand { command } => {
            handle_set_ide_command(resources, command.clone());
        }
        EditorAction::EditMaterial { guid, material } => {
            handle_edit_material(resources, *guid, material);
        }
        EditorAction::ImportAssets { files, dest } => handle_import_assets(resources, files, dest),
        // ECS actions and Undo/Redo handled by caller.
        _ => {}
    }
}

/// Copies each source file into `dest`, then forces a project asset
/// re-scan so the new files register (and get `.meta` sidecars) and
use assets::{handle_edit_material, handle_import_assets};
use play::{handle_play, handle_stop};
use project::{
    handle_clean_project, handle_close_project, handle_create_project, handle_launch_project,
    handle_open_project, handle_remove_recent,
};
use remote::handle_rebuild_remote;
use scene::{
    handle_close_scene, handle_open_scene, handle_open_scene_additive, handle_save_scene,
    handle_set_active_scene,
};
use settings::{
    handle_cancel_launch, handle_reparent, handle_set_ide_command, handle_set_power_profile,
};

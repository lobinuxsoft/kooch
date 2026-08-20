//! Handlers for non-ECS [`EditorAction`] variants.
//!
//! Split by the part of the editor each one moves — assets, scenes,
//! play, the project, the remote session, settings — with the dispatcher
//! here. `remote` is deliberately its own file: it is the half that goes
//! away when the remote protocol is demolished, and keeping it separate
//! makes that a deletion rather than a surgery.

mod assets;
mod play;
mod prefab;
mod project;
mod remote;
mod scene;
mod settings;

use kooch_core::resource::Resources;

use crate::undo::UndoStack;

use crate::actions::EditorAction;

// The remote path resolves a prefab's destination with the same rules as
// the local one, rather than a second copy that could disagree about where
// prefabs live.
pub(crate) use assets::{persist_asset, write_material};
pub(crate) use prefab::PendingHostReloads;
pub(crate) use prefab::{
    asset_saved, entity_name, prefab_path, prefab_saved, project_root as prefab_root,
};

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
        // The scene's undo is the caller's business in local mode and the
        // wire's in remote mode; a document's is neither.
        EditorAction::Undo(document) | EditorAction::Redo(document) if !document.is_world() => {
            crate::history::documents::step(
                resources,
                document,
                matches!(action, EditorAction::Undo(_)),
            );
        }
        EditorAction::CopyEntities(entities) => handle_copy(resources, entities),
        EditorAction::SaveScene => handle_save_scene(resources),
        // Removing the pending prompt already happened in `apply_actions`;
        // there is nothing left for this to do.
        EditorAction::CancelPrefabOverwrite => {}
        // Local mode has one cache and it was refreshed on save.
        EditorAction::ReloadAssetOnHost(_) => {}
        EditorAction::RevertToPrefab { entity, component } => {
            if let Some((root, overrides, writes)) =
                crate::actions::prefab_propagate::plan_revert(resources, *entity, *component)
            {
                crate::actions::prefab_propagate::apply(resources, &writes, &[]);
                crate::actions::prefab_propagate::write_overrides(resources, root, &overrides);
            }
        }
        EditorAction::PropagatePrefab(prefab) => {
            let (writes, removals) = crate::actions::prefab_propagate::plan(resources, *prefab);
            crate::actions::prefab_propagate::apply(resources, &writes, &removals);
        }
        EditorAction::EditPrefabField {
            prefab: guid,
            entity_index,
            component,
            field,
            value,
        } => prefab::handle_edit_prefab_field(
            resources,
            *guid,
            *entity_index,
            component,
            field,
            value.clone(),
        ),
        EditorAction::EditPrefabComponent {
            prefab: guid,
            entity_index,
            component,
            add,
        } => {
            prefab::handle_edit_prefab_component(resources, *guid, *entity_index, *component, *add)
        }
        EditorAction::SavePrefabAsset(guid) => prefab::handle_save_prefab_asset(resources, *guid),
        EditorAction::SavePrefab { entity, dest, .. } => {
            prefab::handle_save_prefab(resources, *entity, dest.as_deref())
        }
        EditorAction::InstantiatePrefab { prefab: guid, at } => {
            prefab::handle_instantiate_prefab(resources, *guid, *at)
        }
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
        EditorAction::UpdateEngine => handle_update_engine(resources),
        EditorAction::MoveProjectToEngine(path) => handle_move_project_to_engine(resources, &path),
        EditorAction::KeepEngine => handle_keep_engine(resources),
        EditorAction::RemoveEngine(version) => handle_remove_engine(version),
        EditorAction::SetIdeCommand { command } => {
            handle_set_ide_command(resources, command.clone());
        }
        EditorAction::SetLaunchEnv { value } => {
            handle_set_launch_env(resources, value.clone());
        }
        EditorAction::EditMaterial {
            guid,
            material,
            commit,
        } => {
            handle_edit_material(resources, *guid, material, *commit);
        }
        EditorAction::SetImageImport { guid, import } => {
            handle_set_image_import(resources, *guid, *import);
        }
        EditorAction::EditAssetField {
            guid,
            field,
            value,
            commit,
        } => {
            handle_edit_asset_field(resources, *guid, field, value.clone(), *commit);
        }
        EditorAction::ImportAssets { files, dest } => handle_import_assets(resources, files, dest),
        // ECS actions and Undo/Redo handled by caller.
        _ => {}
    }
}

/// Fills the clipboard from the selection.
///
/// The one handler that is the same in both modes: reading an entity is
/// reading the local ECS whether that ECS is the world or a mirror of
/// one, and nothing is sent anywhere. `Copy` is the only edit-menu
/// command that never touches the project.
///
/// An empty selection leaves the clipboard alone rather than clearing it.
/// Ctrl+C with nothing selected is a miss, and a miss should not cost the
/// user what they copied a minute ago.
fn handle_copy(resources: &mut Resources, entities: &[kooch_ecs::entity::Entity]) {
    if entities.is_empty() {
        return;
    }
    let states: Vec<_> = entities
        .iter()
        .map(|entity| crate::actions::entity_state::capture(resources, *entity))
        .collect();
    if resources
        .get::<crate::clipboard::EntityClipboard>()
        .is_none()
    {
        resources.insert(crate::clipboard::EntityClipboard::default());
    }
    if let Some(clipboard) = resources.get_mut::<crate::clipboard::EntityClipboard>() {
        tracing::debug!(
            target: "kooch_editor_core::clipboard",
            entities = states.len(),
            "copied",
        );
        clipboard.set(states);
    }
}

/// Copies each source file into `dest`, then forces a project asset
/// re-scan so the new files register (and get `.meta` sidecars) and
use assets::{
    handle_edit_asset_field, handle_edit_material, handle_import_assets, handle_set_image_import,
};
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
    handle_cancel_launch, handle_keep_engine, handle_move_project_to_engine, handle_remove_engine,
    handle_reparent, handle_set_ide_command, handle_set_launch_env, handle_update_engine,
};

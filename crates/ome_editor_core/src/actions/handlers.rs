//! Handlers for non-ECS [`super::EditorAction`] variants — scene I/O,
//! play/stop, project lifecycle, reparenting, recent-projects bookkeeping,
//! and the power-profile slot. Each one mutates `Resources` directly
//! and is not undoable.

use std::any::TypeId;

use ome_core::power::PowerProfile;
use ome_core::resource::Resources;
use ome_ecs::EphemeralComponents;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::Parent;

use crate::play_state::PlayState;
use crate::project_state::ProjectState;
use crate::state::EditorOverlay;
use crate::undo::UndoStack;

use super::EditorAction;
use super::reparent::rewrite_local_transform_for_reparent;
use super::scene_io::{load_scene, save_scene_as};

/// Dispatches a non-ECS, non-undo action to the appropriate handler.
/// ECS actions (`Spawn`, `Despawn`, `SetField`, `AddComponent`,
/// `RemoveComponent`) plus `Undo` / `Redo` are handled by the caller —
/// this function is a no-op for them.
pub(super) fn apply_non_ecs_action(
    action: &EditorAction,
    resources: &mut Resources,
    undo_stack: &mut UndoStack,
) {
    match action {
        EditorAction::SaveScene => handle_save_scene(resources),
        EditorAction::OpenScene => handle_open_scene(resources, undo_stack),
        EditorAction::Play => handle_play(resources),
        EditorAction::Stop => handle_stop(resources),
        EditorAction::OpenProject(path) => handle_open_project(resources, path),
        EditorAction::CreateProject { name, parent_path } => {
            handle_create_project(resources, name, parent_path);
        }
        EditorAction::CloseProject => handle_close_project(resources, undo_stack),
        EditorAction::Reparent { entity, new_parent } => {
            handle_reparent(resources, *entity, *new_parent);
        }
        EditorAction::RemoveRecent(path) => handle_remove_recent(resources, path),
        EditorAction::LaunchProject(path) => handle_launch_project(resources, path),
        EditorAction::CancelLaunch => handle_cancel_launch(resources),
        EditorAction::SetPowerProfile(profile) => handle_set_power_profile(resources, *profile),
        // ECS actions and Undo/Redo handled by caller.
        _ => {}
    }
}

fn handle_save_scene(resources: &mut Resources) {
    let scenes_dir = resources
        .get::<ProjectState>()
        .and_then(|ps| ps.active_project.as_ref().map(|p| p.root_path.join("scenes")));
    let mut dialog = rfd::FileDialog::new().add_filter("OME Scene", &["ome_scene"]);
    if let Some(ref dir) = scenes_dir {
        dialog = dialog.set_directory(dir);
    }
    let Some(path) = dialog.save_file() else { return };
    match save_scene_as(resources, path.clone()) {
        Ok(()) => tracing::info!("scene saved to {}", path.display()),
        Err(e) => tracing::error!("failed to save scene: {e}"),
    }
}

fn handle_open_scene(resources: &mut Resources, undo_stack: &mut UndoStack) {
    let scenes_dir = resources
        .get::<ProjectState>()
        .and_then(|ps| ps.active_project.as_ref().map(|p| p.root_path.join("scenes")));
    let mut dialog = rfd::FileDialog::new().add_filter("OME Scene", &["ome_scene"]);
    if let Some(ref dir) = scenes_dir {
        dialog = dialog.set_directory(dir);
    }
    let Some(path) = dialog.pick_file() else { return };
    match load_scene(resources, &path) {
        Ok(()) => {
            tracing::info!("scene loaded from {}", path.display());
            undo_stack.clear();
        }
        Err(e) => tracing::error!("failed to load scene: {e}"),
    }
}

fn handle_play(resources: &mut Resources) {
    let (manifest_path, engine_root) = match resources.get::<ProjectState>() {
        Some(ps) => (
            ps.active_project
                .as_ref()
                .map(|p| p.root_path.join("Cargo.toml")),
            ps.engine_root.clone(),
        ),
        None => (None, None),
    };
    let Some(manifest_path) = manifest_path else {
        tracing::error!("Play: no active project — open a project first");
        return;
    };
    if !manifest_path.exists() {
        tracing::error!(
            "Play: project has no Cargo.toml at {} — Play only works on crate-projects",
            manifest_path.display()
        );
        return;
    }
    let doc = ome_ecs::SceneDocument::from_ecs(resources);
    let scene_path = std::env::temp_dir().join("ome_play_scene.ome_scene");
    if let Err(e) = doc.save(&scene_path) {
        tracing::error!("failed to save play scene: {e}");
    } else if let Some(play_state) = resources.get_mut::<PlayState>()
        && let Err(e) =
            play_state.launch(&manifest_path, &scene_path, engine_root.as_deref())
    {
        tracing::error!("failed to launch game: {e}");
    }
}

fn handle_stop(resources: &mut Resources) {
    if let Some(play_state) = resources.get_mut::<PlayState>() {
        play_state.stop();
    }
}

fn handle_open_project(resources: &mut Resources, path: &std::path::Path) {
    let Some(mut ps) = resources.remove::<ProjectState>() else { return };
    match ps.open_project(path) {
        Ok(()) => {
            let title = ps
                .active_project
                .as_ref()
                .map(|p| p.manifest.name.clone())
                .unwrap_or_default();
            tracing::info!("opened project: {title}");

            if let Some(wh) = resources.get::<ome_window::WindowHandle>() {
                let _ = wh.window().set_title(&format!("{title} — Oh My Engine"));
            }

            let main_scene_path = ps.active_project.as_ref().and_then(|p| {
                p.manifest
                    .main_scene
                    .as_ref()
                    .map(|s| p.root_path.join(s))
            });
            if let Some(scene_path) = main_scene_path
                && scene_path.exists()
                && let Err(e) = load_scene(resources, &scene_path)
            {
                tracing::error!("failed to load main scene: {e}");
            }

            ps.show_new_project_form = false;
        }
        Err(e) => {
            tracing::error!("failed to open project: {e}");
            ps.new_project_form.error = Some(format!("Failed to open project: {e}"));
        }
    }
    resources.insert(ps);
}

fn handle_create_project(
    resources: &mut Resources,
    name: &str,
    parent_path: &std::path::Path,
) {
    let engine_root = resources
        .get::<ProjectState>()
        .and_then(|ps| ps.engine_root.clone());

    let Some(engine_root) = engine_root else {
        tracing::error!("engine_root not set — cannot create project crate");
        if let Some(ps) = resources.get_mut::<ProjectState>() {
            ps.new_project_form.error = Some("Engine root not configured".to_owned());
        }
        return;
    };

    match crate::project::create_project(name, parent_path, &engine_root) {
        Ok(root) => {
            tracing::info!("created project: {name}");
            if let Some(ps) = resources.get_mut::<ProjectState>() {
                ps.editor_config.add_recent(name, &root);
                if let Err(e) = ps.editor_config.save() {
                    tracing::warn!("failed to save editor config: {e}");
                }
                ps.show_new_project_form = false;
                ps.spawn_launcher(&root);
            }
        }
        Err(e) => {
            tracing::error!("failed to create project: {e}");
            if let Some(ps) = resources.get_mut::<ProjectState>() {
                ps.new_project_form.error = Some(format!("Failed to create project: {e}"));
            }
        }
    }
}

fn handle_close_project(resources: &mut Resources, undo_stack: &mut UndoStack) {
    // Respect the ephemeral marker registry — editor-owned entities
    // (camera, gizmo helpers, …) carry an `EditorOnly`-style marker
    // and must survive the close so the next project open finds
    // them already spawned. `spawn_editor_camera_system` is a Stage::
    // Startup one-shot; without this filter the close path despawned
    // the camera and reopening the project left the viewport with
    // only the project's gameplay camera, no editor controls.
    let ephemeral = resources
        .get::<EphemeralComponents>()
        .cloned()
        .unwrap_or_default();
    let target_entities: Vec<Entity> = resources
        .get::<ArchetypeRegistry>()
        .map(|archetypes| {
            archetypes
                .iter_matching(&[])
                .filter(|arch| !ephemeral.intersects(arch.components()))
                .flat_map(|a| a.entities().to_vec())
                .collect()
        })
        .unwrap_or_default();

    for entity in &target_entities {
        if let Some(alloc) = resources.get_mut::<EntityAllocator>() {
            alloc.despawn(*entity);
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.unregister_entity(*entity);
        }
        if let Some(components) = resources.get_mut::<ComponentRegistry>() {
            components.remove_entity(*entity);
        }
    }

    if let Some(overlay) = resources.get_mut::<EditorOverlay>() {
        overlay.selected_entities.clear();
        overlay.last_clicked_index = None;
    }

    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.close_project();
    }
    if let Some(wh) = resources.get::<ome_window::WindowHandle>() {
        let _ = wh.window().set_title("Oh My Engine");
    }

    undo_stack.clear();
}

fn handle_reparent(resources: &mut Resources, entity: Entity, new_parent: Option<Entity>) {
    // Preserve the child's world-space transform across the reparent.
    // Without this, parenting snaps the child to `parent * child_local`
    // and unparenting snaps it back to `child_local` (as if it were a
    // root all along).
    rewrite_local_transform_for_reparent(resources, entity, new_parent);

    match new_parent {
        Some(parent) => {
            let mut needs_archetype_add = false;
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                let has_parent = registry
                    .get_cpu::<Parent>()
                    .is_some_and(|s| s.contains(entity));
                if has_parent {
                    if let Some(storage) = registry.get_cpu_mut::<Parent>()
                        && let Some(p) = storage.get_mut(entity)
                    {
                        p.entity = parent;
                    }
                } else if let Some(storage) = registry.get_cpu_mut::<Parent>() {
                    storage.insert(entity, Parent { entity: parent });
                    needs_archetype_add = true;
                }
            }
            if needs_archetype_add {
                let parent_tid = TypeId::of::<Parent>();
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
                    && let Some(current) = archetypes.entity_archetype(entity)
                {
                    let new_arch = archetypes.archetype_after_add_dynamic(current, parent_tid);
                    archetypes.register_entity(entity, new_arch);
                }
            }
        }
        None => {
            let parent_tid = TypeId::of::<Parent>();
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.remove_component(entity, &parent_tid);
            }
            if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
                && let Some(current) = archetypes.entity_archetype(entity)
            {
                let new_arch = archetypes.archetype_after_remove_dynamic(current, parent_tid);
                archetypes.register_entity(entity, new_arch);
            }
        }
    }
}

fn handle_remove_recent(resources: &mut Resources, path: &std::path::Path) {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.editor_config.remove_recent(path);
        if let Err(e) = ps.editor_config.save() {
            tracing::warn!("failed to save editor config: {e}");
        }
    }
}

fn handle_launch_project(resources: &mut Resources, path: &std::path::Path) {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        ps.editor_config.add_recent(&name, path);
        if let Err(e) = ps.editor_config.save() {
            tracing::warn!("failed to save editor config: {e}");
        }
        ps.spawn_launcher(path);
    }
}

fn handle_cancel_launch(resources: &mut Resources) {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.kill_launcher();
    }
}

fn handle_set_power_profile(resources: &mut Resources, profile: PowerProfile) {
    if let Some(slot) = resources.get_mut::<PowerProfile>() {
        if *slot != profile {
            tracing::info!(from = slot.as_str(), to = profile.as_str(), "power profile changed");
            *slot = profile;
        }
    } else {
        resources.insert(profile);
    }
}

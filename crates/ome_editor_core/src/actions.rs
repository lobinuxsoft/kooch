//! Editor actions collected during UI, applied after render.

use std::any::TypeId;
use std::path::PathBuf;

use glam::{Quat, Vec3};
use ome_core::resource::Resources;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::Parent;
use ome_ecs::reflect::ReflectValue;
use ome_ecs::transform::Transform;

use crate::play_state::PlayState;
use crate::project_state::ProjectState;
use crate::state::EditorOverlay;
use crate::undo::{
    AddComponentCommand, CompoundCommand, DespawnCommand, EditorCommand, RemoveComponentCommand,
    SetFieldCommand, SpawnCommand, UndoStack,
};

pub(crate) enum EditorAction {
    /// Spawn an entity with Name + Transform + optional extra components.
    /// The optional String sets the Name component value.
    Spawn {
        extra: Vec<TypeId>,
        name: Option<String>,
    },
    Despawn(Entity),
    SetField {
        entity: Entity,
        type_id: TypeId,
        field: String,
        value: ReflectValue,
    },
    AddComponent {
        entity: Entity,
        type_id: TypeId,
    },
    RemoveComponent {
        entity: Entity,
        type_id: TypeId,
    },
    Undo,
    Redo,
    SaveScene,
    OpenScene,
    Play,
    Stop,
    OpenProject(PathBuf),
    CreateProject {
        name: String,
        parent_path: PathBuf,
    },
    CloseProject,
    Reparent {
        entity: Entity,
        new_parent: Option<Entity>,
    },
    RemoveRecent(PathBuf),
    LaunchProject(PathBuf),
    CancelLaunch,
}

/// Converts an action into an undoable command, capturing before-state.
///
/// Returns `None` for non-ECS actions (scene I/O, play, project management).
fn action_to_command(
    action: &EditorAction,
    resources: &Resources,
) -> Option<Box<dyn EditorCommand>> {
    match action {
        EditorAction::Spawn { extra, name } => {
            Some(Box::new(SpawnCommand::new(extra.clone(), name.clone())))
        }
        EditorAction::Despawn(entity) => Some(Box::new(DespawnCommand::new(resources, *entity))),
        EditorAction::SetField {
            entity,
            type_id,
            field,
            value,
        } => {
            if let Some(cmd) =
                SetFieldCommand::new(resources, *entity, *type_id, field.clone(), value.clone())
            {
                Some(Box::new(cmd))
            } else {
                tracing::warn!("failed to create SetFieldCommand for '{field}'");
                None
            }
        }
        EditorAction::AddComponent { entity, type_id } => {
            Some(Box::new(AddComponentCommand::new(*entity, *type_id)))
        }
        EditorAction::RemoveComponent { entity, type_id } => {
            Some(Box::new(RemoveComponentCommand::new(resources, *entity, *type_id)))
        }
        _ => None,
    }
}

/// Returns a description for a group of same-variant actions.
fn batch_description(actions: &[EditorAction]) -> String {
    let count = actions.len();
    match actions.first() {
        Some(EditorAction::Spawn { .. }) => format!("Spawn {count} Entities"),
        Some(EditorAction::Despawn(_)) => format!("Despawn {count} Entities"),
        Some(EditorAction::SetField { .. }) => format!("Set {count} Fields"),
        Some(EditorAction::AddComponent { .. }) => format!("Add {count} Components"),
        Some(EditorAction::RemoveComponent { .. }) => format!("Remove {count} Components"),
        _ => "Batch".to_owned(),
    }
}

/// Returns `true` if two actions are the same ECS variant (ignoring payload).
fn same_ecs_variant(a: &EditorAction, b: &EditorAction) -> bool {
    matches!(
        (a, b),
        (EditorAction::Spawn { .. }, EditorAction::Spawn { .. })
            | (EditorAction::Despawn(_), EditorAction::Despawn(_))
            | (EditorAction::SetField { .. }, EditorAction::SetField { .. })
            | (EditorAction::AddComponent { .. }, EditorAction::AddComponent { .. })
            | (EditorAction::RemoveComponent { .. }, EditorAction::RemoveComponent { .. })
    )
}

pub(crate) fn apply_actions(
    resources: &mut Resources,
    actions: &[EditorAction],
    undo_stack: &mut UndoStack,
) {
    let mut i = 0;
    while i < actions.len() {
        let action = &actions[i];

        // Undo/Redo are handled directly.
        if matches!(action, EditorAction::Undo) {
            undo_stack.undo(resources);
            i += 1;
            continue;
        }
        if matches!(action, EditorAction::Redo) {
            undo_stack.redo(resources);
            i += 1;
            continue;
        }

        // Check if this is an ECS action that can be batched.
        if action_to_command(action, resources).is_some() {
            // Find the run of consecutive same-variant ECS actions.
            let run_start = i;
            let mut run_end = i + 1;
            while run_end < actions.len() && same_ecs_variant(action, &actions[run_end]) {
                run_end += 1;
            }
            let run = &actions[run_start..run_end];

            if run.len() == 1 {
                // Single action — execute directly (snapshot already captured above
                // was discarded; re-capture since resources may have changed).
                if let Some(cmd) = action_to_command(&run[0], resources) {
                    undo_stack.execute(cmd, resources);
                }
            } else {
                // Multiple same-type actions — batch into a CompoundCommand.
                let desc = batch_description(run);
                let mut cmds: Vec<Box<dyn EditorCommand>> = Vec::with_capacity(run.len());
                for a in run {
                    // Snapshot must be taken sequentially: each command's
                    // before-state depends on the previous command's execution.
                    if let Some(cmd) = action_to_command(a, resources) {
                        cmds.push(cmd);
                    }
                }
                let compound = CompoundCommand::new(desc, cmds);
                undo_stack.execute(Box::new(compound), resources);
            }

            i = run_end;
            continue;
        }

        // Non-ECS actions: process directly (no undo).
        match action {
            EditorAction::SaveScene => {
                let scenes_dir = resources
                    .get::<ProjectState>()
                    .and_then(|ps| ps.active_project.as_ref().map(|p| p.root_path.join("scenes")));
                let mut dialog = rfd::FileDialog::new()
                    .add_filter("OME Scene", &["ome_scene"]);
                if let Some(ref dir) = scenes_dir {
                    dialog = dialog.set_directory(dir);
                }
                let path = dialog.save_file();
                if let Some(path) = path {
                    let doc = ome_ecs::SceneDocument::from_ecs(resources);
                    if let Err(e) = doc.save(&path) {
                        tracing::error!("failed to save scene: {e}");
                    } else {
                        tracing::info!("scene saved to {}", path.display());
                    }
                }
            }
            EditorAction::OpenScene => {
                let scenes_dir = resources
                    .get::<ProjectState>()
                    .and_then(|ps| ps.active_project.as_ref().map(|p| p.root_path.join("scenes")));
                let mut dialog = rfd::FileDialog::new()
                    .add_filter("OME Scene", &["ome_scene"]);
                if let Some(ref dir) = scenes_dir {
                    dialog = dialog.set_directory(dir);
                }
                let path = dialog.pick_file();
                if let Some(path) = path {
                    match ome_ecs::SceneDocument::load(&path) {
                        Ok(doc) => {
                            if let Err(e) = ome_ecs::sync_scene_to_ecs(&doc, resources) {
                                tracing::error!("failed to sync scene to ECS: {e}");
                            } else {
                                tracing::info!("scene loaded from {}", path.display());
                                undo_stack.clear();
                            }
                        }
                        Err(e) => {
                            tracing::error!("failed to load scene: {e}");
                        }
                    }
                }
            }
            EditorAction::Play => {
                let doc = ome_ecs::SceneDocument::from_ecs(resources);
                let scene_path = std::env::temp_dir().join("ome_play_scene.ome_scene");
                if let Err(e) = doc.save(&scene_path) {
                    tracing::error!("failed to save play scene: {e}");
                } else {
                    let is_project = resources
                        .get::<ProjectState>()
                        .is_some_and(|ps| {
                            if ps.is_project_binary {
                                return true;
                            }
                            let Some(project) = &ps.active_project else { return false };
                            let Ok(exe) = std::env::current_exe() else { return false };
                            exe.starts_with(project.root_path.join("target"))
                        });
                    let exe = is_project
                        .then(|| std::env::current_exe().ok())
                        .flatten();
                    if let Some(play_state) = resources.get_mut::<PlayState>() {
                        if let Err(e) = play_state.launch(&scene_path, exe.as_deref()) {
                            tracing::error!("failed to launch game: {e}");
                        }
                    }
                }
            }
            EditorAction::Stop => {
                if let Some(play_state) = resources.get_mut::<PlayState>() {
                    play_state.stop();
                }
            }
            EditorAction::OpenProject(path) => {
                let mut ps = match resources.remove::<ProjectState>() {
                    Some(ps) => ps,
                    None => {
                        i += 1;
                        continue;
                    }
                };
                match ps.open_project(path) {
                    Ok(()) => {
                        let title = ps
                            .active_project
                            .as_ref()
                            .map(|p| p.manifest.name.clone())
                            .unwrap_or_default();
                        tracing::info!("opened project: {title}");

                        if let Some(wh) = resources.get::<ome_window::WindowHandle>() {
                            let _ = wh
                                .window()
                                .set_title(&format!("{title} — Oh My Engine"));
                        }

                        let main_scene_path = ps.active_project.as_ref().and_then(|p| {
                            p.manifest
                                .main_scene
                                .as_ref()
                                .map(|s| p.root_path.join(s))
                        });
                        if let Some(scene_path) = main_scene_path {
                            if scene_path.exists() {
                                match ome_ecs::SceneDocument::load(&scene_path) {
                                    Ok(doc) => {
                                        if let Err(e) =
                                            ome_ecs::sync_scene_to_ecs(&doc, resources)
                                        {
                                            tracing::error!(
                                                "failed to sync main scene: {e}"
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!("failed to load main scene: {e}");
                                    }
                                }
                            }
                        }

                        ps.show_new_project_form = false;
                    }
                    Err(e) => {
                        tracing::error!("failed to open project: {e}");
                        ps.new_project_form.error =
                            Some(format!("Failed to open project: {e}"));
                    }
                }
                resources.insert(ps);
            }
            EditorAction::CreateProject { name, parent_path } => {
                let engine_root = resources
                    .get::<ProjectState>()
                    .and_then(|ps| ps.engine_root.clone());

                let Some(engine_root) = engine_root else {
                    tracing::error!("engine_root not set — cannot create project crate");
                    if let Some(ps) = resources.get_mut::<ProjectState>() {
                        ps.new_project_form.error =
                            Some("Engine root not configured".to_owned());
                    }
                    i += 1;
                    continue;
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
                            ps.new_project_form.error =
                                Some(format!("Failed to create project: {e}"));
                        }
                    }
                }
            }
            EditorAction::CloseProject => {
                let all_entities: Vec<Entity> = resources
                    .get::<ArchetypeRegistry>()
                    .map(|archetypes| {
                        archetypes
                            .iter_matching(&[])
                            .flat_map(|a| a.entities().to_vec())
                            .collect()
                    })
                    .unwrap_or_default();

                for entity in &all_entities {
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
            EditorAction::Reparent { entity, new_parent } => {
                // Preserve the child's world-space transform across the
                // reparent. Without this, parenting snaps the child to
                // `parent * child_local` and unparenting snaps it back
                // to `child_local` (as if it were a root all along).
                rewrite_local_transform_for_reparent(resources, *entity, *new_parent);

                match new_parent {
                    Some(parent) => {
                        let mut needs_archetype_add = false;
                        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                            let has_parent = registry
                                .get_cpu::<Parent>()
                                .is_some_and(|s| s.contains(*entity));
                            if has_parent {
                                if let Some(storage) = registry.get_cpu_mut::<Parent>() {
                                    if let Some(p) = storage.get_mut(*entity) {
                                        p.entity = *parent;
                                    }
                                }
                            } else if let Some(storage) = registry.get_cpu_mut::<Parent>() {
                                storage.insert(*entity, Parent { entity: *parent });
                                needs_archetype_add = true;
                            }
                        }
                        if needs_archetype_add {
                            let parent_tid = TypeId::of::<Parent>();
                            if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                                if let Some(current) = archetypes.entity_archetype(*entity) {
                                    let new_arch = archetypes
                                        .archetype_after_add_dynamic(current, parent_tid);
                                    archetypes.register_entity(*entity, new_arch);
                                }
                            }
                        }
                    }
                    None => {
                        let parent_tid = TypeId::of::<Parent>();
                        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                            registry.remove_component(*entity, &parent_tid);
                        }
                        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                            if let Some(current) = archetypes.entity_archetype(*entity) {
                                let new_arch = archetypes
                                    .archetype_after_remove_dynamic(current, parent_tid);
                                archetypes.register_entity(*entity, new_arch);
                            }
                        }
                    }
                }
            }
            EditorAction::RemoveRecent(path) => {
                if let Some(ps) = resources.get_mut::<ProjectState>() {
                    ps.editor_config.remove_recent(path);
                    if let Err(e) = ps.editor_config.save() {
                        tracing::warn!("failed to save editor config: {e}");
                    }
                }
            }
            EditorAction::LaunchProject(path) => {
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
            EditorAction::CancelLaunch => {
                if let Some(ps) = resources.get_mut::<ProjectState>() {
                    ps.kill_launcher();
                }
            }
            // ECS actions and Undo/Redo are already handled above.
            _ => {}
        }
        i += 1;
    }
}

/// Rewrites an entity's local `Transform` so its world-space pose
/// stays the same across a reparent. Call this BEFORE updating the
/// entity's `Parent` component.
///
/// Works entirely in TRS space (translation / rotation / scale) by
/// walking up the hierarchy chain on both sides and composing per
/// component. This avoids the `Mat4::to_scale_rotation_translation`
/// SVD pass that the previous implementation did — that path drifts
/// precision on every reparent when any ancestor carries rotation,
/// which compounded across multiple drag cycles into visibly
/// corrupted numbers in the inspector.
///
/// Silently does nothing if the entity or any ancestor lacks a
/// `Transform` — in that case the previous snap-back behavior is
/// unavoidable and matches the old impl.
fn rewrite_local_transform_for_reparent(
    resources: &mut Resources,
    entity: Entity,
    new_parent: Option<Entity>,
) {
    let Some((child_wp, child_wr, child_ws)) = compute_world_trs(resources, entity) else {
        return;
    };
    let (parent_wp, parent_wr, parent_ws) = match new_parent {
        Some(p) => compute_world_trs(resources, p)
            .unwrap_or((Vec3::ZERO, Quat::IDENTITY, Vec3::ONE)),
        None => (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE),
    };

    // Derive the local TRS that satisfies
    //   new_parent.world ⊕ new_local = child.world
    // where `⊕` is the Unity-style compose:
    //   world_pos = parent_pos + parent_rot · (parent_scale * local_pos)
    //   world_rot = parent_rot · local_rot
    //   world_scale = parent_scale * local_scale  (component-wise)
    let parent_rot_inv = parent_wr.inverse();
    let inv_parent_scale = Vec3::new(
        safe_inv(parent_ws.x),
        safe_inv(parent_ws.y),
        safe_inv(parent_ws.z),
    );
    // Inverse of `T + R · (S ⊙ local)`: subtract T, apply R⁻¹, then
    // divide by S component-wise. Doing the scale division before the
    // rotation was the order bug that corrupted reparents when the
    // parent had both rotation and non-uniform scale.
    let new_local_pos = (parent_rot_inv * (child_wp - parent_wp)) * inv_parent_scale;
    let new_local_rot = parent_rot_inv * child_wr;
    let new_local_scale = child_ws * inv_parent_scale;

    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(transform_storage) = registry.get_cpu_mut::<Transform>()
        && let Some(transform) = transform_storage.get_mut(entity)
    {
        transform.position = new_local_pos;
        transform.rotation = new_local_rot;
        transform.scale = new_local_scale;
    }
}

/// Walks up the parent chain from `entity` to a root, composing TRS
/// per component. Returns the world-space `(translation, rotation,
/// scale)` or `None` if the entity has no `Transform`.
///
/// Intentionally avoids touching `GlobalTransform.matrix` because
/// that path depends on the SVD decomposition used during propagation,
/// which loses precision for hierarchies that mix rotation and
/// non-uniform scale. Walking TRS directly keeps reparent math stable.
fn compute_world_trs(
    resources: &Resources,
    entity: Entity,
) -> Option<(Vec3, Quat, Vec3)> {
    let registry = resources.get::<ComponentRegistry>()?;
    let transform_storage = registry.get_cpu::<Transform>()?;
    let parent_storage = registry.get_cpu::<Parent>();

    // Collect ancestry from `entity` up to the root.
    let mut chain = Vec::with_capacity(8);
    chain.push(entity);
    let mut current = entity;
    while let Some(parent) = parent_storage.as_ref().and_then(|s| s.get(current)) {
        if chain.contains(&parent.entity) {
            // Cycle in the hierarchy. Bail.
            break;
        }
        chain.push(parent.entity);
        current = parent.entity;
    }
    chain.reverse(); // root first

    // Fold TRS down the chain.
    let mut world_pos = Vec3::ZERO;
    let mut world_rot = Quat::IDENTITY;
    let mut world_scale = Vec3::ONE;
    for &e in &chain {
        let t = transform_storage.get(e)?;
        let new_pos = world_pos + world_rot * (world_scale * t.position);
        let new_rot = world_rot * t.rotation;
        let new_scale = world_scale * t.scale;
        world_pos = new_pos;
        world_rot = new_rot;
        world_scale = new_scale;
    }
    Some((world_pos, world_rot, world_scale))
}

/// Inverse with a floor to avoid division by zero on degenerate scales.
fn safe_inv(v: f32) -> f32 {
    if v.abs() < 1e-6 { 1.0 / 1e-6 } else { 1.0 / v }
}


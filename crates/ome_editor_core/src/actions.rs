//! Editor actions collected during UI, applied after render.

use std::any::TypeId;
use std::path::PathBuf;

use ome_core::resource::Resources;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::commands::Commands;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::reflect::ReflectValue;

use crate::play_state::PlayState;
use crate::project_state::ProjectState;
use crate::state::EditorOverlay;

pub(crate) enum EditorAction {
    Spawn,
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
    RemoveRecent(PathBuf),
    LaunchProject(PathBuf),
    CancelLaunch,
}

pub(crate) fn apply_actions(resources: &mut Resources, actions: &[EditorAction]) {
    for action in actions {
        match action {
            EditorAction::Spawn => {
                let mut commands = match resources.remove::<Commands>() {
                    Some(c) => c,
                    None => return,
                };
                let entity = commands.spawn(resources).id();
                resources.insert(commands);

                // Auto-add Name and Transform defaults for editor-spawned entities.
                let default_components: Vec<TypeId> = resources
                    .get::<ComponentRegistry>()
                    .map(|reg| {
                        reg.reflected_type_names()
                            .into_iter()
                            .filter(|(_, name)| {
                                let short = name.rsplit("::").next().unwrap_or(name);
                                short == "Name" || short == "Transform"
                            })
                            .map(|(tid, _)| tid)
                            .collect()
                    })
                    .unwrap_or_default();

                for type_id in &default_components {
                    let mut inserted = false;
                    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                        inserted = registry.insert_default_reflected(type_id, entity);
                    }
                    if inserted {
                        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                            if let Some(current) = archetypes.entity_archetype(entity) {
                                let new_arch =
                                    archetypes.archetype_after_add_dynamic(current, *type_id);
                                archetypes.register_entity(entity, new_arch);
                            }
                        }
                    }
                }
            }
            EditorAction::Despawn(entity) => {
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
            EditorAction::SetField {
                entity,
                type_id,
                field,
                value,
            } => {
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    if let Err(e) =
                        registry.reflect_set_field(type_id, *entity, field, value.clone())
                    {
                        tracing::warn!("failed to set field '{field}': {e}");
                    }
                }
            }
            EditorAction::AddComponent { entity, type_id } => {
                let mut inserted = false;
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    inserted = registry.insert_default_reflected(type_id, *entity);
                }
                if inserted {
                    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                        if let Some(current) = archetypes.entity_archetype(*entity) {
                            let new_arch =
                                archetypes.archetype_after_add_dynamic(current, *type_id);
                            archetypes.register_entity(*entity, new_arch);
                        }
                    }
                }
            }
            EditorAction::RemoveComponent { entity, type_id } => {
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    registry.remove_component(*entity, type_id);
                }
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    if let Some(current) = archetypes.entity_archetype(*entity) {
                        let new_arch =
                            archetypes.archetype_after_remove_dynamic(current, *type_id);
                        archetypes.register_entity(*entity, new_arch);
                    }
                }
            }
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
                    // In project mode, use current_exe() to avoid recompilation.
                    let is_project = resources
                        .get::<ProjectState>()
                        .is_some_and(|ps| ps.is_project_binary);
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
                // Remove ProjectState to avoid borrow conflicts with resources.
                let mut ps = match resources.remove::<ProjectState>() {
                    Some(ps) => ps,
                    None => continue,
                };
                match ps.open_project(path) {
                    Ok(()) => {
                        let title = ps
                            .active_project
                            .as_ref()
                            .map(|p| p.manifest.name.clone())
                            .unwrap_or_default();
                        tracing::info!("opened project: {title}");

                        // Set window title.
                        if let Some(wh) = resources.get::<ome_window::WindowHandle>() {
                            let _ = wh
                                .window()
                                .set_title(&format!("{title} — Oh My Engine"));
                        }

                        // Load main_scene if present.
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
                    continue;
                };

                match crate::project::create_project(name, parent_path, &engine_root) {
                    Ok(root) => {
                        tracing::info!("created project: {name}");
                        // Add to recents and launch the project binary.
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
                // Despawn all entities.
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

                // Clear selection.
                if let Some(overlay) = resources.get_mut::<EditorOverlay>() {
                    overlay.selected_entities.clear();
                    overlay.last_clicked_index = None;
                }

                // Close project and reset title.
                if let Some(ps) = resources.get_mut::<ProjectState>() {
                    ps.close_project();
                }
                if let Some(wh) = resources.get::<ome_window::WindowHandle>() {
                    let _ = wh.window().set_title("Oh My Engine");
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
                    // Add to recents before launching.
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
        }
    }
}


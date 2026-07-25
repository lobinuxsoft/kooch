//! Handlers for non-ECS [`super::EditorAction`] variants — scene I/O,
//! play/stop, project lifecycle, reparenting, recent-projects bookkeeping,
//! and the power-profile slot. Each one mutates `Resources` directly
//! and is not undoable.

use std::any::TypeId;

use ome_core::Guid;
use ome_core::asset_database::AssetDatabase;
use ome_core::asset_loader::AssetServer;
use ome_core::assets::Assets;
use ome_core::power::PowerProfile;
use ome_core::resource::Resources;
use ome_ecs::EphemeralComponents;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::Parent;
use ome_render::material::Material;

use crate::play_state::PlayState;
use crate::project_state::ProjectState;
use crate::remote_session::{RemoteSession, RemoteState};
use crate::state::EditorOverlay;
use crate::undo::UndoStack;

use super::EditorAction;
use super::scene_io::{load_scene, save_scene_as, scene_dialog};

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
    if super::asset_ops::handle_asset_op(action, resources) {
        return;
    }
    match action {
        EditorAction::SaveScene => handle_save_scene(resources),
        EditorAction::OpenScene => handle_open_scene(resources, undo_stack),
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
/// surface in the Asset Browser + pickers next frame.
fn handle_import_assets(
    resources: &mut Resources,
    files: &[std::path::PathBuf],
    dest: &std::path::Path,
) {
    if let Err(e) = std::fs::create_dir_all(dest) {
        tracing::error!(dest = %dest.display(), error = %e, "import: cannot create destination");
        return;
    }
    let mut copied = 0usize;
    for src in files {
        let Some(name) = src.file_name() else {
            continue;
        };
        let target = super::asset_ops::unique_target(dest, name);
        match std::fs::copy(src, &target) {
            Ok(_) => {
                copied += 1;
                tracing::info!(from = %src.display(), to = %target.display(), "asset imported");
            }
            Err(e) => {
                tracing::error!(from = %src.display(), error = %e, "asset import failed");
            }
        }
    }
    if copied > 0 {
        super::asset_ops::force_rescan(resources);
    }
}

/// Applies a Material asset edit: updates `Assets<Material>` in place so
/// the render sync uploads the new params live, then serialises the
/// material back to its source `.ron` so the change survives a reload.
fn handle_edit_material(resources: &mut Resources, guid: Guid, material: &Material) {
    // 1. Live update: resolve the GUID to a handle (loading if needed)
    //    and overwrite the stored asset.
    let Some(mut server) = resources.remove::<AssetServer>() else {
        tracing::warn!("EditMaterial: AssetServer missing; edit dropped");
        return;
    };
    let handle = server.load_by_guid::<Material>(guid, resources);
    resources.insert(server);
    match handle {
        Ok(h) => {
            if let Some(assets) = resources.get_mut::<Assets<Material>>()
                && let Some(slot) = assets.get_mut(h)
            {
                *slot = material.clone();
            }
        }
        Err(e) => {
            tracing::warn!(guid = %guid, error = %e, "EditMaterial: failed to resolve material")
        }
    }

    // 2. Persist to disk at the asset's registered path.
    let Some(path) = resources
        .get::<AssetDatabase>()
        .and_then(|db| db.entry(guid).map(|e| e.path.clone()))
    else {
        tracing::warn!(guid = %guid, "EditMaterial: no path in AssetDatabase; not persisted");
        return;
    };
    match ron::ser::to_string_pretty(material, ron::ser::PrettyConfig::default()) {
        Ok(text) => match std::fs::write(&path, text) {
            Ok(()) => tracing::info!(path = %path.display(), "material saved"),
            Err(e) => {
                tracing::error!(path = %path.display(), error = %e, "failed to write material")
            }
        },
        Err(e) => tracing::error!(guid = %guid, error = %e, "failed to serialise material"),
    }
}

fn handle_save_scene(resources: &mut Resources) {
    let Some(path) = scene_dialog(resources).save_file() else {
        return;
    };
    match save_scene_as(resources, path.clone()) {
        Ok(()) => tracing::info!("scene saved to {}", path.display()),
        Err(e) => tracing::error!("failed to save scene: {e}"),
    }
}

fn handle_open_scene(resources: &mut Resources, undo_stack: &mut UndoStack) {
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
        && let Err(e) = play_state.launch(&manifest_path, &scene_path, engine_root.as_deref())
    {
        tracing::error!("failed to launch game: {e}");
    }
}

fn handle_stop(resources: &mut Resources) {
    if let Some(play_state) = resources.get_mut::<PlayState>() {
        play_state.stop();
    }
}

/// Opens a project — that is, launches it and starts driving it.
///
/// A project is a Rust crate that owns its own component types, so the
/// hub cannot meaningfully load its scene itself: half of every entity
/// would arrive as a parked component with no behaviour behind it. It
/// launches the project in `--remote` mode instead and mirrors the world
/// the project owns, which is also what makes Play run gameplay in the
/// editor's viewport rather than in a second window.
///
/// The in-process path survives only for a folder with no crate to run.
/// Such a project can be inspected but not played.
fn handle_open_project(resources: &mut Resources, path: &std::path::Path) {
    if !path.join("Cargo.toml").exists() {
        tracing::warn!(
            project = %path.display(),
            "no Cargo.toml — opening read-only, without a running project"
        );
        open_project(resources, path, SceneSource::LocalFile);
        return;
    }
    open_project(resources, path, SceneSource::RemoteMirror);
    start_remote_session(resources);
}

/// Where the opened project's entities come from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SceneSource {
    /// Load the manifest's main scene into the editor's own ECS.
    LocalFile,
    /// Leave the ECS empty — a remote session will mirror the project's
    /// live world into it once the project's server answers.
    RemoteMirror,
}

fn open_project(resources: &mut Resources, path: &std::path::Path, scene: SceneSource) {
    let Some(mut ps) = resources.remove::<ProjectState>() else {
        return;
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
                let _ = wh.window().set_title(&format!("{title} — Oh My Engine"));
            }

            let main_scene_path = ps
                .active_project
                .as_ref()
                .and_then(|p| p.manifest.main_scene.as_ref().map(|s| p.root_path.join(s)));
            if let Some(scene_path) = main_scene_path
                && scene == SceneSource::LocalFile
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

/// Opens a project in remote mode: the project's own binary owns the ECS
/// and the editor becomes a client of it.
///
/// The scene is deliberately **not** loaded locally — this binary has no
/// Rust types for the project's components, so a local load would park
/// half of every entity. The project loads its own scene at boot and the
/// mirror pulls it in once connected.
/// Rebuilds the project and reconnects to the fresh binary.
///
/// The only way to pick up code the project did not have when it
/// started: Rust is compiled ahead of time, so a new component or system
/// needs a rebuild, and the running process cannot grow one. Also the
/// way back from a session that died — the launch is a `cargo run`, so
/// it recompiles and relaunches in one step.
fn handle_rebuild_remote(resources: &mut Resources) {
    disconnect_remote(resources);
    start_remote_session(resources);
}

/// Launches the active project in remote mode and adopts the session.
///
/// Regenerates `src/registrations.rs` first. That file is editor-owned,
/// and a project last registered by an older editor still gates its
/// systems at build time — Play would flip a `Playing` gate nothing
/// reads. Rewriting it before the build is what makes an existing
/// project pick up the runtime gate without the user knowing it exists.
fn start_remote_session(resources: &mut Resources) {
    super::register_scripts(resources);

    let Some((manifest_path, engine_root)) = resources.get::<ProjectState>().and_then(|ps| {
        let project = ps.active_project.as_ref()?;
        Some((project.root_path.join("Cargo.toml"), ps.engine_root.clone()))
    }) else {
        tracing::error!("remote: no active project");
        return;
    };
    if !manifest_path.exists() {
        tracing::error!(
            manifest = %manifest_path.display(),
            "remote: no Cargo.toml — remote mode only works on crate-projects"
        );
        return;
    }

    match RemoteSession::launch(&manifest_path, engine_root.as_deref()) {
        Ok(session) => {
            if let Some(state) = resources.get_mut::<RemoteState>() {
                state.session = Some(session);
                state.playing = false;
            }
            // Reset the cadence so a stale failure from a previous
            // session does not suppress this one's reporting.
            if let Some(sync) = resources.get_mut::<crate::systems::RemoteSyncState>() {
                *sync = Default::default();
            }
        }
        Err(e) => tracing::error!("remote: failed to launch project: {e}"),
    }
}

/// Ends any remote session and tears its mirror out of the ECS.
///
/// Mirrored entities are ephemeral, so the ordinary close sweep skips
/// them; without this they would outlive the project that owns them.
fn disconnect_remote(resources: &mut Resources) {
    let Some(mut state) = resources.remove::<RemoteState>() else {
        return;
    };
    if let Some(session) = state.session.as_mut() {
        session.stop();
    }
    state.session = None;
    state.playing = false;
    state.mirror.clear(resources);
    resources.insert(state);
}

fn handle_create_project(resources: &mut Resources, name: &str, parent_path: &std::path::Path) {
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
    // Before the sweep: a remote session outlives its project otherwise,
    // and its mirrored entities are ephemeral so the sweep skips them.
    disconnect_remote(resources);

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
    // Moved to ome_ecs::hierarchy (#595): the server has to be able to
    // perform this too, and while it lived here remote mode had no way to
    // reparent at all.
    ome_ecs::hierarchy::reparent(resources, entity, new_parent);
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

fn handle_set_ide_command(resources: &mut Resources, command: Option<String>) {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.editor_config.ide_command = command;
        if let Err(e) = ps.editor_config.save() {
            tracing::warn!(error = %e, "failed to save editor config");
        }
    }
}

fn handle_set_power_profile(resources: &mut Resources, profile: PowerProfile) {
    if let Some(slot) = resources.get_mut::<PowerProfile>() {
        if *slot != profile {
            tracing::info!(
                from = slot.as_str(),
                to = profile.as_str(),
                "power profile changed"
            );
            *slot = profile;
        }
    } else {
        resources.insert(profile);
    }
}

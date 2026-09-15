//! Opening, creating and closing a project, plus the recent list and
//! launching a built one.

use kooch_core::resource::Resources;
use kooch_ecs::EphemeralComponents;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;

use crate::project_state::ProjectState;
use crate::state::EditorOverlay;
use crate::undo::UndoStack;

use super::remote::{disconnect_remote, start_remote_session};
use crate::actions::scene_io::load_scene;

pub(super) fn handle_open_project(resources: &mut Resources, path: &std::path::Path) {
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

            if let Some(wh) = resources.get::<kooch_window::WindowHandle>() {
                let _ = wh
                    .window()
                    .set_title(&crate::bootstrap::window_title(Some(&title)));
            }

            // Bring an older project's layout up to date *before* looking
            // for its library: a project that predates the library split
            // has none, and migrating first is what gives it one to build.
            if let Some(root) = ps.active_project.as_ref().map(|p| p.root_path.clone()) {
                let crate_name = crate::project::sanitize_crate_name(&title);
                crate::actions::migrate_to_library(&root, &crate_name);
                // Then split authoring out of the game build (#558). After the library migration,
                // not before: this adds a second `[[bin]]`, and the one above is what stops cargo
                // inferring the first from `src/main.rs`.
                crate::actions::split_authoring(&root, &crate_name);
                // What `#import kooch::surface` resolves to in an editor like VS Code (#1158).
                crate::shader_sync::write_surface_api(&root);

                // Then load it, if it has been built. Writing lib.rs does not produce a .so — that
                // needs a compile — so the first open after a migration finds nothing and says so
                // rather than leaving the menu quietly short.
                let gained =
                    crate::project_plugin::load_project_plugin(resources, &root, &crate_name);
                if gained > 0 {
                    tracing::info!(types = gained, "project components available in the editor");
                } else if crate::project_plugin::library_path(&root, &crate_name).is_none() {
                    tracing::info!(
                        project = %root.display(),
                        "this project has not been built yet — build it and reopen to see its own components"
                    );
                }
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

/// Opens a project in remote mode: the project's own binary owns the ECS and the editor becomes a
/// client of it.

pub(super) fn handle_create_project(
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
            }
            // Open it the way Open Project opens one, rather than handing control to the new
            // project's own embedded editor and exiting.
            handle_open_project(resources, &root);
        }
        Err(e) => {
            tracing::error!("failed to create project: {e}");
            if let Some(ps) = resources.get_mut::<ProjectState>() {
                ps.new_project_form.error = Some(format!("Failed to create project: {e}"));
            }
        }
    }
}

/// Runs `cargo clean` on the open project.
pub(super) fn handle_clean_project(resources: &mut Resources) {
    let Some(root) = resources
        .get::<ProjectState>()
        .and_then(|ps| ps.active_project.as_ref().map(|ap| ap.root_path.clone()))
    else {
        tracing::warn!("clean: no project open");
        return;
    };

    disconnect_remote(resources);

    let before = directory_size(&root.join("target"));
    match std::process::Command::new("cargo")
        .arg("clean")
        .current_dir(&root)
        .output()
    {
        Ok(output) if output.status.success() => {
            let freed = before.saturating_sub(directory_size(&root.join("target")));
            tracing::info!(
                freed_mb = freed / 1_048_576,
                "cleaned the project — press Rebuild to build it again",
            );
        }
        Ok(output) => {
            // cargo's own words: a broken manifest is the usual reason,
            // and paraphrasing it would hide which line.
            tracing::error!(
                status = ?output.status.code(),
                stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                "cargo clean failed",
            );
        }
        Err(e) => tracing::error!("could not run cargo clean: {e}"),
    }
}

/// Bytes under `path`, or zero if it is missing or unreadable.
fn directory_size(path: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => directory_size(&entry.path()),
            Ok(_) => entry.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

pub(super) fn handle_close_project(resources: &mut Resources, undo_stack: &mut UndoStack) {
    // Before the sweep: a remote session outlives its project otherwise,
    // and its mirrored entities are ephemeral so the sweep skips them.
    disconnect_remote(resources);

    // Respect the ephemeral marker registry — editor-owned entities (camera, gizmo helpers, …)
    // carry an `EditorOnly`-style marker and must survive the close so the next project open finds
    // them already spawned.
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
        // Pins name entities from the world that just went away. Entity ids are generational, so a
        // stale one cannot match a new entity — but keeping them would grow the set for the life of
        // the session with ids nothing will ever draw.
        overlay.pinned_gizmos.clear();
        overlay.last_clicked_index = None;
    }

    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.close_project();
    }
    if let Some(wh) = resources.get::<kooch_window::WindowHandle>() {
        let _ = wh.window().set_title(&crate::bootstrap::window_title(None));
    }

    undo_stack.clear();
    // The remote history describes the world of the project being closed.
    // Left behind, the next project opened would offer to undo edits made
    // to the last one, against ids it has no idea about.
    if let Some(history) = resources.get_mut::<crate::actions::remote_undo::RemoteHistory>() {
        history.clear();
    }
}

pub(super) fn handle_remove_recent(resources: &mut Resources, path: &std::path::Path) {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.editor_config.remove_recent(path);
        if let Err(e) = ps.editor_config.save() {
            tracing::warn!("failed to save editor config: {e}");
        }
    }
}

pub(super) fn handle_launch_project(resources: &mut Resources, path: &std::path::Path) {
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

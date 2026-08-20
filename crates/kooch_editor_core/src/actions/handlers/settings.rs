//! Reparenting, the IDE command, the power profile, and cancelling a
//! launch — the handlers that do not belong to a larger subsystem.

use kooch_core::power::PowerProfile;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use std::path::Path;

use crate::project_state::ProjectState;

pub(super) fn handle_reparent(
    resources: &mut Resources,
    entity: Entity,
    new_parent: Option<Entity>,
) {
    // Moved to kooch_ecs::hierarchy (#595): the server has to be able to
    // perform this too, and while it lived here remote mode had no way to
    // reparent at all.
    kooch_ecs::hierarchy::reparent(resources, entity, new_parent);
}

pub(super) fn handle_cancel_launch(resources: &mut Resources) {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.kill_launcher();
    }
}

/// Installs the engine this editor ships, answering the notice.
pub(super) fn handle_update_engine(resources: &mut Resources) {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.update_engine();
    }
}

/// Dismisses the notice without touching what is installed.
pub(super) fn handle_keep_engine(resources: &mut Resources) {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.keep_engine();
    }
}

pub(super) fn handle_set_ide_command(resources: &mut Resources, command: Option<String>) {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.editor_config.ide_command = command;
        if let Err(e) = ps.editor_config.save() {
            tracing::warn!(error = %e, "failed to save editor config");
        }
    }
}

/// Records the launch environment against the OPEN project's path.
///
/// Nothing happens with no project open, which is also when the field
/// that raises this is not drawn: a line stored against no path could
/// only ever apply to everything or to nothing.
pub(super) fn handle_set_launch_env(resources: &mut Resources, value: String) {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        let Some(root) = ps.active_project.as_ref().map(|p| p.root_path.clone()) else {
            return;
        };
        ps.editor_config.set_launch_env(&root, value);
        if let Err(e) = ps.editor_config.save() {
            tracing::warn!(error = %e, "failed to save editor config");
        }
    }
}

pub(super) fn handle_set_power_profile(resources: &mut Resources, profile: PowerProfile) {
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

/// Deletes an installed engine.
///
/// The version this editor ships is refused inside `remove_engine`, and
/// the panel does not offer the button for it or for the one the open
/// project uses — belt and braces, because what it deletes is a
/// directory a manifest may be naming.
pub(super) fn handle_remove_engine(version: &str) {
    match crate::engine_vendor::remove_engine(version) {
        Ok(()) => tracing::info!(version, "removed an installed engine"),
        Err(e) => tracing::warn!(version, error = %e, "could not remove the engine"),
    }
}

/// Moves a project onto this editor's engine from the launcher, without
/// opening it.
///
/// 🔴 The whole point is what it does **not** do. Opening a project
/// compiles its plugin and only then compares engine versions, so a
/// mismatch costs a full compile against the engine being left behind,
/// and the `.so` that comes out is refused by `BuildStamp`. Settled
/// here, the first compile is already against the right engine (#800).
pub(super) fn handle_move_project_to_engine(resources: &mut Resources, project_root: &Path) {
    let version = crate::engine_vendor::editor_engine_version();
    let source = resources
        .get::<ProjectState>()
        .and_then(|ps| crate::engine_vendor::vendor_source(ps.engine_root.as_deref()));

    // The engine has to exist before a project can point at it. Already
    // materialised is the common case and costs nothing.
    let engine_dir = match crate::engine_vendor::ensure_current(version, source.as_deref()) {
        Ok((_, Some(dir))) => dir,
        Ok((_, None)) => {
            tracing::warn!("no engine source available to move the project onto");
            return;
        }
        Err(e) => {
            tracing::warn!("could not materialise the engine: {e}");
            return;
        }
    };

    match crate::project::move_project_to_engine(project_root, &engine_dir, version) {
        Ok(()) => tracing::info!(
            project = %project_root.display(),
            version,
            "project moved onto this editor's engine — its next build is a full rebuild",
        ),
        Err(e) => tracing::warn!("could not move the project onto the engine: {e}"),
    }
}

//! Everything that exists only because the editor drives the project over a socket: rebuilding the
//! session and starting it.

use kooch_core::resource::Resources;

use crate::project_state::ProjectState;
use crate::remote_session::{RemoteSession, RemoteState};

pub(super) fn handle_rebuild_and_run(resources: &mut Resources) {
    // 🔴 Before the teardown, and that is the whole point. The world belongs to the project's
    // process; killing it drops every edit made since the last save and reopens whichever scene the
    // project starts with.
    let held = crate::carry::capture(resources);
    if held > 0 {
        tracing::info!(scenes = held, "holding the world across the rebuild");
    }
    disconnect_remote(resources);
    start_remote_session(resources);
}

/// Launches the active project in remote mode and adopts the session.
pub(super) fn start_remote_session(resources: &mut Resources) {
    crate::actions::register_scripts(resources);

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
                // A fresh attempt, so the banner does not show the
                // previous build's output above this one's.
                state.connect_output.clear();
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
pub(super) fn disconnect_remote(resources: &mut Resources) {
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

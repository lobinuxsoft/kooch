//! [`RemotePlugin`] — starts the server and drains it each tick.

use ome_core::app::App;
use ome_core::frame_pacing::{FramePace, FrameRequest, FrameWaker};
use ome_core::plugin::Plugin;
use ome_core::resource::Resources;
use ome_core::run_state::Playing;
use ome_core::stage::Stage;

use crate::handlers::handle;
use crate::server::RemoteServer;

/// Adds the remote editor server to a running project.
///
/// Binds a loopback HTTP port on a dedicated thread and installs a
/// [`Stage::First`] system that answers queued requests against the ECS.
/// A bind failure is logged and the plugin becomes inert rather than
/// aborting the app — a project should still run if the port is taken.
pub struct RemotePlugin {
    /// Socket name to bind, or `None` to read it from the environment.
    name: Option<String>,
}

impl RemotePlugin {
    /// The plugin on the socket name the launcher passed down.
    ///
    /// Reads [`NAME_ENV`](crate::NAME_ENV), falling back to
    /// [`DEFAULT_NAME`](crate::DEFAULT_NAME) so a project run by hand
    /// still works.
    pub fn new() -> Self {
        Self { name: None }
    }

    /// The plugin on a specific socket name.
    pub fn on_socket(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
        }
    }
}

impl Default for RemotePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for RemotePlugin {
    fn build(&self, app: &mut App) {
        // Present since `App::new`; cloned rather than borrowed because
        // the listener thread outlives this call.
        let waker = app
            .resources
            .get::<FrameWaker>()
            .cloned()
            .unwrap_or_default();
        let started = match &self.name {
            Some(name) => RemoteServer::start_waking(name, waker),
            None => RemoteServer::start_from_env(waker),
        };
        match started {
            Ok(server) => {
                app.insert_resource(server);
                // #656 — a project under an editor is not a game running:
                // between edits nothing simulates, and a frame that
                // nobody asked for is a core spent mirroring a still
                // scene. The baseline sleeps; the socket wakes it, and
                // Play overrides it for as long as Play lasts. A project
                // launched without this plugin never gets a
                // `FrameRequest` and keeps spinning, as a game should.
                app.insert_resource(FrameRequest::new(FramePace::Wait));
                // First stage: apply remote edits before this frame's
                // systems observe the world, so a client edit lands the
                // same tick it arrives.
                app.add_system(Stage::First, serve_pending_system);
                // Last, so it sees the flag the frame ended with rather
                // than the one it started with — pressing Play must not
                // cost a frame of latency at the far end of a socket.
                app.add_system(Stage::Last, pace_system);
            }
            Err(e) => {
                tracing::warn!("remote editor server disabled: {e}");
            }
        }
    }
}

/// Keeps the loop spinning while the project is playing.
///
/// Only ever raises the pace: a frame that is simulating needs the next
/// one, and nothing else in the frame gets to say otherwise.
fn pace_system(resources: &mut Resources) {
    if Playing::is_playing(resources) {
        FrameRequest::raise(resources, FramePace::Continuous);
    }
}

/// Executes every queued request and replies to each waiting listener.
fn serve_pending_system(resources: &mut Resources) {
    let Some(pending) = resources
        .get::<RemoteServer>()
        .map(|server| server.take_pending())
    else {
        return;
    };
    if pending.is_empty() {
        return;
    }

    for item in pending {
        let response = handle(&item.request, resources);
        // A failed send means the listener stopped waiting (client hung
        // up); drop the response and move on.
        let _ = item.reply.send(response);
    }
}

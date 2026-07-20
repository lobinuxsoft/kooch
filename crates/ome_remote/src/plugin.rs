//! [`RemotePlugin`] — starts the server and drains it each tick.

use ome_core::app::App;
use ome_core::plugin::Plugin;
use ome_core::resource::Resources;
use ome_core::stage::Stage;

use crate::handlers::handle;
use crate::server::{DEFAULT_PORT, RemoteServer};

/// Adds the remote editor server to a running project.
///
/// Binds a loopback HTTP port on a dedicated thread and installs a
/// [`Stage::First`] system that answers queued requests against the ECS.
/// A bind failure is logged and the plugin becomes inert rather than
/// aborting the app — a project should still run if the port is taken.
pub struct RemotePlugin {
    port: u16,
}

impl RemotePlugin {
    /// The plugin on the [`DEFAULT_PORT`].
    pub fn new() -> Self {
        Self { port: DEFAULT_PORT }
    }

    /// The plugin on a specific port.
    pub fn on_port(port: u16) -> Self {
        Self { port }
    }
}

impl Default for RemotePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for RemotePlugin {
    fn build(&self, app: &mut App) {
        match RemoteServer::start(self.port) {
            Ok(server) => {
                app.insert_resource(server);
                // First stage: apply remote edits before this frame's
                // systems observe the world, so a client edit lands the
                // same tick it arrives.
                app.add_system(Stage::First, serve_pending_system);
            }
            Err(e) => {
                tracing::warn!("remote editor server disabled: {e}");
            }
        }
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

//! Streams a game's profiler frames over TCP to the editor or `puffin_viewer` (#785): the frame
//! that matters is a game on the OneXFly at 10 W. Behind the off-by-default `profiling` feature —
//! otherwise absent (#558).

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::stage::Stage;

// The address both ends agree on lives in `kooch_core` because the panel
// that connects to this server cannot see this crate — the facade depends
// on the editor, not the reverse.
pub use kooch_core::profiler::{ADDR_VAR, DEFAULT_PORT, default_bind_addr};

/// Streams profiler frames to whoever connects; added by [`DefaultPlugins`](crate::DefaultPlugins)
/// when `profiling` is on, so games need no `main.rs` edit.
pub struct ProfilingPlugin {
    /// Listen address. 🔴 `0.0.0.0`, not loopback: the handheld is the one machine not running the
    /// viewer, and loopback just times out.
    pub bind_addr: String,
}

impl Default for ProfilingPlugin {
    fn default() -> Self {
        Self {
            bind_addr: default_bind_addr(),
        }
    }
}

/// Keeps the server alive with the app. 🔴 `puffin_http::Server` closes on drop (`#[must_use]`),
/// leaving a game listening on nothing, silently.
struct ProfilerServer(puffin_http::Server);

impl Plugin for ProfilingPlugin {
    fn build(&self, app: &mut App) {
        // Records from the first frame: nobody presses Record on a handheld over SSH.
        puffin::set_scopes_on(true);

        match puffin_http::Server::new(&self.bind_addr) {
            Ok(server) => {
                tracing::info!(
                    addr = %self.bind_addr,
                    "profiler listening; connect the editor's Profiler panel or `puffin_viewer --url <host>:{DEFAULT_PORT}`"
                );
                // 🔴 A server learns scope names only from the frame registering them (`scope_delta`
                // is a delta), so earlier scopes show as `scope#ScopeId(67)`. One snapshot at
                // startup makes it independent of plugin order.
                puffin::GlobalProfiler::lock().emit_scope_snapshot();
                app.insert_resource(ProfilerServer(server));
            }
            // A taken port logs and the game still runs — usually two builds left running on the
            // device.
            Err(err) => {
                tracing::error!(
                    addr = %self.bind_addr,
                    %err,
                    "profiler could not listen; set {ADDR_VAR} to a free address"
                );
            }
        }

        // 🔴 Exactly one frame boundary: two give half-frames, none a frame that never ends. In
        // `Last`, since headless and winit runners both run stages. ⚠️ The editor marks its own and
        // does not add this plugin.
        app.add_system(Stage::Last, |_| {
            profiling::finish_frame!();
        });
    }
}

/// Connected viewers, `None` if the server never started — "is anyone listening", beside the logged
/// address.
pub fn connected_viewers(app: &App) -> Option<usize> {
    app.resources
        .get::<ProfilerServer>()
        .map(|server| server.0.num_clients())
}

#[cfg(test)]
mod tests;

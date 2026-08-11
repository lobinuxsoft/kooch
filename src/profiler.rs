//! Serving a running game's frames to a profiler over the network (#785).
//!
//! Everything the profiler panel has measured so far describes the editor,
//! on a desktop, plugged in. The number the graphics roadmap is judged
//! against is a frame of a *game* on the OneXFly at 10 W, and no amount of
//! measuring this machine produces it. This plugin is the other end of
//! that: a game build that streams its frames over TCP, which the editor —
//! or `puffin_viewer` — connects to from the desktop.
//!
//! # It is not in a release build
//!
//! The whole module is behind the `profiling` cargo feature, off by
//! default. With the feature off there is no listening socket, no
//! background thread, and every `profiling::scope!` in the engine expands
//! to nothing — not "disabled", absent. That is what #558 asks of a
//! shipped build, and it is why the profiling build is its own preset
//! rather than a runtime switch.

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::stage::Stage;

/// Port the server listens on when nothing else is asked for.
///
/// 8585 is puffin's own default, so `puffin_viewer` connects with no
/// arguments beyond the address.
pub const DEFAULT_PORT: u16 = 8585;

/// Environment variable that overrides the bind address, e.g.
/// `KOOCH_PROFILER_ADDR=0.0.0.0:9000`.
///
/// Present so a build already copied onto the device can be moved off a
/// taken port without compiling a new one.
pub const ADDR_VAR: &str = "KOOCH_PROFILER_ADDR";

/// Streams this process's profiler frames to anyone who connects.
///
/// Added to [`DefaultPlugins`](crate::DefaultPlugins) automatically when
/// the `profiling` feature is on, so a game gains it by being built with
/// the profiling preset rather than by editing its `main.rs`.
pub struct ProfilingPlugin {
    /// Address to listen on.
    ///
    /// 🔴 `0.0.0.0`, not `127.0.0.1`. Bound to loopback the game is only
    /// reachable from the handheld itself, which is the one machine that
    /// will not be running the viewer, and the symptom is a connection
    /// that times out with nothing logged anywhere.
    pub bind_addr: String,
}

impl Default for ProfilingPlugin {
    fn default() -> Self {
        let bind_addr =
            std::env::var(ADDR_VAR).unwrap_or_else(|_| format!("0.0.0.0:{DEFAULT_PORT}"));
        Self { bind_addr }
    }
}

/// Keeps the server alive for as long as the app is.
///
/// 🔴 The only reason this resource exists. `puffin_http::Server` closes
/// its socket and unregisters its sink on drop — it is `#[must_use]` for
/// exactly that reason — so a server built and dropped inside `build`
/// leaves a game that listens on nothing and reports no error.
struct ProfilerServer(puffin_http::Server);

impl Plugin for ProfilingPlugin {
    fn build(&self, app: &mut App) {
        // The editor's profiler starts stopped, because someone is sitting
        // in front of it deciding when to record. Nobody is going to press
        // Record on a handheld over SSH, so this one records from the
        // first frame.
        puffin::set_scopes_on(true);

        match puffin_http::Server::new(&self.bind_addr) {
            Ok(server) => {
                tracing::info!(
                    addr = %self.bind_addr,
                    "profiler listening; connect the editor's Profiler panel or `puffin_viewer --url <host>:{DEFAULT_PORT}`"
                );
                app.insert_resource(ProfilerServer(server));
            }
            // A game that cannot open the port still runs. The port being
            // taken is the common case — two builds of the same game left
            // running on the device — and killing the process the user
            // wanted to measure is a worse answer than saying so.
            Err(err) => {
                tracing::error!(
                    addr = %self.bind_addr,
                    %err,
                    "profiler could not listen; set {ADDR_VAR} to a free address"
                );
            }
        }

        // 🔴 The frame boundary, and there is exactly one of it.
        //
        // puffin builds a frame out of the scopes that closed between two
        // `new_frame` calls, so two boundaries per frame produce a
        // flamegraph of half-frames and none at all produces one frame
        // that grows forever and never renders.
        //
        // It goes in `Last` rather than in a runner because there are two
        // runners — `kooch_core::runner::default_runner` for a headless
        // app and `kooch_window`'s winit loop for a windowed one — and a
        // stage runs under both.
        //
        // ⚠️ `kooch_editor_core` marks its own boundary, in
        // `systems/render/ui.rs`. An app that somehow ran both would be
        // the two-boundary case above; the editor does not add this
        // plugin, and if it ever does, that call goes away in the same
        // commit.
        app.add_system(Stage::Last, |_| {
            profiling::finish_frame!();
        });
    }
}

/// Number of viewers currently connected, or `None` if the server never
/// came up.
///
/// The game logs the address it listens on, which answers "is it
/// serving"; this answers "is anyone listening", which is the question
/// asked while staring at a viewer that shows nothing.
pub fn connected_viewers(app: &App) -> Option<usize> {
    app.resources
        .get::<ProfilerServer>()
        .map(|server| server.0.num_clients())
}

#[cfg(test)]
mod tests;

//! kooch_core - Core functionality for Kooch
//!
//! Provides the foundation for the game engine:
//! - [`App`] - Application struct with builder pattern
//! - [`Plugin`] - Modular functionality system
//! - [`Schedule`] - System organization by execution stage
//! - [`Resources`] - Type-erased global state storage
//! - [`Events`] - Double-buffered event system
//! - [`Time`] - Frame timing with fixed timestep support
//!
//! # Quick Start
//! ```ignore
//! use kooch_core::prelude::*;
//!
//! fn startup(resources: &mut Resources) {
//!     tracing::info!("Game started!");
//! }
//!
//! fn update(resources: &mut Resources) {
//!     if let Some(time) = resources.get::<Time>() {
//!         // Game logic using delta time
//!     }
//! }
//!
//! App::new()
//!     .add_plugins(MinimalPlugins)
//!     .add_system(Stage::Startup, startup)
//!     .add_system(Stage::Update, update)
//!     .run();
//! ```
//!
//! # Game Loop
//! The default runner implements "Fix Your Timestep":
//! - Variable-rate frame stages (rendering adapts to display)
//! - Fixed-rate physics stages (deterministic simulation)
//!
//! ```text
//! Startup → [First → Input → PreUpdate → Update → PostUpdate →
//!            GpuSync → Gpu → (Physics → PostPhysics)* →
//!            PreRender → Render → PostRender → Last] → repeat
//!
//! * Physics stages run N times per frame to catch up to real time
//! ```

// Re-exported because `AssetMeta::import` and
// `LoadContext::with_import` both carry a `toml::Table` in their public
// signatures: a crate that writes an importer already depends on this
// type, and making it add the dependency by hand is how two versions of
// the same parser end up in one build.
pub use toml;

pub mod aabb;
pub mod app;
pub mod asset_database;
pub mod asset_loader;
pub mod asset_meta;
pub mod asset_registry;
pub mod assets;
pub mod buffer;
pub mod compute;
pub mod coord;
pub mod event;
pub mod frame_metrics;
pub mod frame_pacing;
pub mod gpu;
pub mod guid;
pub mod log_console;
pub mod pipeline_cache;
pub mod plugin;
pub mod prelude;
pub mod profiler;
pub mod raw_event;
pub mod resource;
pub mod run_state;
pub mod runner;
pub mod scene_paths;
pub mod schedule;
/// Whether this build carries a profiling scope per system.
///
/// 🔴 Exists to be asserted by the crates that select the feature. A
/// profiling build whose scopes compiled out is not a build that fails —
/// it is a build whose captures quietly look like the ones from before
/// the scopes existed, with `PreUpdate` reporting one number and no
/// children. That already happened once, between adding the scopes and
/// noticing the editor never enabled them.
pub const CPU_SCOPES: bool = cfg!(feature = "cpu-profiler");

pub mod stage;
pub mod system;
pub mod time;
pub mod window_mode;

/// Re-exported so `register_asset!` resolves from any crate.
#[doc(hidden)]
pub use inventory;

pub use aabb::Aabb;
pub use guid::Guid;

#[cfg(feature = "dynamic")]
pub mod dynamic;

/// Initializes the tracing subscriber for logging.
///
/// Call this early in your application (before creating the App) if you
/// want to see log output. Uses the `RUST_LOG` environment variable for
/// filtering (defaults to `info`).
///
/// # Example
/// ```ignore
/// fn main() {
///     kooch_core::init_tracing();
///     App::new().add_plugins(MinimalPlugins).run();
/// }
/// ```
pub use log_console::{LogBuffer, LogEntry, strip_ansi};

/// Installs tracing with a console buffer beside stdout, and hands the
/// buffer back.
///
/// For a host with a UI to show the log in. Everything written through
/// `tracing` reaches both, so the panel and the terminal cannot disagree
/// about what happened — including a spawned project's output, which the
/// editor forwards through `tracing` already.
pub fn init_tracing_with_console() -> LogBuffer {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let buffer = LogBuffer::new();

    tracing_subscriber::registry()
        .with(fmt::layer().with_ansi(ansi_wanted()))
        .with(buffer.layer())
        .with(filter)
        .init();

    buffer
}

/// Whether stdout is a terminal, and therefore whether colour helps.
///
/// A program writing to a pipe should not emit escape sequences: whoever
/// reads the other end has to strip them, and if they do not, the codes are
/// rendered as glyphs. The editor's Console showed exactly that — the host
/// colourised into a pipe and `\x1b[2m` arrived as boxes.
fn ansi_wanted() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// Whether this process should log as JSON.
///
/// Set by whoever spawned it — the editor does, for a project it hosts.
/// A pre-formatted line is one opaque string to whoever reads the pipe:
/// the editor was wrapping the host's whole formatted line, timestamp and
/// level and all, inside a line of its own, so the Console showed `INFO`
/// twice and could not filter a project's warnings from its info because
/// every forwarded line was, to the editor, an `info` from `render`.
///
/// An env var rather than "not a terminal", because someone doing
/// `cargo run > log.txt` wants a log they can read, not JSON.
fn json_wanted() -> bool {
    std::env::var("KOOCH_LOG_FORMAT").is_ok_and(|v| v.eq_ignore_ascii_case("json"))
}

/// Installs the default subscriber unless the host already installed one.
///
/// Called by [`CorePlugin`](crate::plugin::CorePlugin), so an app that
/// never thought about logging still gets it. That is not a convenience:
/// without a subscriber every `tracing` call in the engine is a no-op, so a
/// host that forgot the line is one whose warnings, errors and events do
/// not exist anywhere — and the way you find out is by not seeing them.
///
/// A generated project's `--game` and `--remote` binaries were exactly
/// that. The remote host logged a joint breaking, a sensor being entered
/// and its own play/stop transitions into nothing at all, and the editor's
/// Console showed a project that never spoke.
///
/// `try_init` rather than `init`: a host that set up its own subscriber —
/// the editor, with its console buffer — keeps it, and this is a no-op.
pub fn init_tracing_if_needed() {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if json_wanted() {
        let _ = tracing_subscriber::registry()
            .with(fmt::layer().json().flatten_event(true))
            .with(filter)
            .try_init();
        return;
    }
    let _ = tracing_subscriber::registry()
        .with(fmt::layer().with_ansi(ansi_wanted()))
        .with(filter)
        .try_init();
}

pub fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(fmt::layer().with_ansi(ansi_wanted()))
        .with(filter)
        .init();
}

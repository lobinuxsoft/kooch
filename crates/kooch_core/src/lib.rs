//! kooch_core - Core functionality for Kooch
//!
//! Provides the foundation for the game engine:
//! - `App` - Application struct with builder pattern
//! - `Plugin` - Modular functionality system
//! - `Schedule` - System organization by execution stage
//! - `Resources` - Type-erased global state storage
//! - `Events` - Double-buffered event system
//! - `Time` - Frame timing with fixed timestep support
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

// Re-exported because `AssetMeta::import` and `LoadContext::with_import` both carry a `toml::Table`
// in their public signatures: a crate that writes an importer already depends on this type, and
// making it add the dependency by hand is how two versions of the same parser end up in one build.
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

pub mod log_file;

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
pub use log_file::{SharedLog, log_panics, open_log};

/// Installs tracing with a console buffer beside stdout, and hands the buffer back.
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
fn ansi_wanted() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// Whether this process should log as JSON.
fn json_wanted() -> bool {
    std::env::var("KOOCH_LOG_FORMAT").is_ok_and(|v| v.eq_ignore_ascii_case("json"))
}

/// Installs the default subscriber unless the host already installed one.
pub fn init_tracing_if_needed() {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // 🔴 The file is what a shipped game can be debugged from (#964). It has no terminal — started
    // from Steam, from a launcher, or by double-click — and under Proton its stdout is not
    // forwarded at all, so without this a Windows build cannot say anything to anybody.
    let file = log_file::open_log();
    if let Some((log, ref path)) = file {
        // Installed before the subscriber, so a panic during setup is
        // still caught — the run this exists for is the one that dies
        // early.
        log_file::log_panics(log.clone());
        let writer = log.clone();
        let installed = match json_wanted() {
            true => tracing_subscriber::registry()
                .with(fmt::layer().json().flatten_event(true))
                .with(
                    fmt::layer()
                        .with_ansi(false)
                        .with_writer(move || writer.clone()),
                )
                .with(filter)
                .try_init(),
            false => tracing_subscriber::registry()
                .with(fmt::layer().with_ansi(ansi_wanted()))
                .with(
                    fmt::layer()
                        .with_ansi(false)
                        .with_writer(move || writer.clone()),
                )
                .with(filter)
                .try_init(),
        };
        if installed.is_ok() {
            // Said once, on the line above everything else, because
            // "where is the log" is the first question anyone asks and
            // the answer differs per platform and per install.
            tracing::info!(path = %path.display(), "logging to file");
        }
        return;
    }

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

//! ome_core - Core functionality for OhMyEngine
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
//! use ome_core::prelude::*;
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

pub mod app;
pub mod compute;
pub mod event;
pub mod gpu;
pub mod plugin;
pub mod prelude;
pub mod resource;
pub mod runner;
pub mod schedule;
pub mod stage;
pub mod time;

/// Initializes the tracing subscriber for logging.
///
/// Call this early in your application (before creating the App) if you
/// want to see log output. Uses the `RUST_LOG` environment variable for
/// filtering (defaults to `info`).
///
/// # Example
/// ```ignore
/// fn main() {
///     ome_core::init_tracing();
///     App::new().add_plugins(MinimalPlugins).run();
/// }
/// ```
pub fn init_tracing() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();
}

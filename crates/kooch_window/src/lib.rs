//! kooch_window - Windowing system for Kooch.
//!
//! Provides a winit-based window with integration into the engine's game loop.
//! The [`WindowPlugin`] replaces the default headless runner with one driven
//! by winit's event loop, enabling real-time rendering and input.
//!
//! # Architecture
//! ```text
//! App::new()
//!   .add_plugins(MinimalPlugins)
//!   .add_plugin(WindowPlugin::default())
//!   .run()
//!
//! → WindowPlugin::build() inserts WindowConfig, registers events, sets runner
//! → winit_runner() creates EventLoop + WinitApp
//! → WinitApp::resumed() creates the window, runs startup systems
//! → WinitApp::window_event(RedrawRequested) drives the frame tick
//! ```
//!
//! # Limitations
//! - Single window only (sufficient for v0.1).

pub mod event;
pub mod handle;
pub mod runner;
mod winit_app;

pub use event::{WindowCloseRequested, WindowResized};
pub use handle::WindowHandle;
pub use runner::winit_runner;

use kooch_core::app::App;
use kooch_core::plugin::Plugin;

/// Configuration for the engine window.
///
/// Inserted as a resource during [`WindowPlugin::build()`] and consumed
/// during `resumed()` to create the actual window. Other plugins can modify
/// this resource before the window is created.
#[derive(Debug, Clone)]
pub struct WindowConfig {
    /// Window title.
    pub title: String,
    /// Window width in logical pixels.
    pub width: u32,
    /// Window height in logical pixels.
    pub height: u32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "Kóoch".to_string(),
            width: 1280,
            height: 720,
        }
    }
}

/// Plugin that creates a window and integrates the engine with winit's event loop.
///
/// # What it does
/// - Inserts [`WindowConfig`] as a resource
/// - Registers [`WindowResized`] and [`WindowCloseRequested`] events
/// - Overrides the runner with [`winit_runner`]
///
/// After `resumed()`, a [`WindowHandle`] resource becomes available for
/// downstream systems (e.g., wgpu surface creation).
///
/// # Example
/// ```ignore
/// use kooch_core::prelude::*;
/// use kooch_window::{WindowPlugin, WindowHandle};
///
/// App::new()
///     .add_plugins(MinimalPlugins)
///     .add_plugin(WindowPlugin {
///         title: "My Game".to_string(),
///         width: 1920,
///         height: 1080,
///     })
///     .run();
/// ```
pub struct WindowPlugin {
    /// Window title.
    pub title: String,
    /// Window width in logical pixels.
    pub width: u32,
    /// Window height in logical pixels.
    pub height: u32,
}

impl Default for WindowPlugin {
    fn default() -> Self {
        let config = WindowConfig::default();
        Self {
            title: config.title,
            width: config.width,
            height: config.height,
        }
    }
}

impl Plugin for WindowPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WindowConfig {
            title: self.title.clone(),
            width: self.width,
            height: self.height,
        });

        app.add_event::<WindowResized>();
        app.add_event::<WindowCloseRequested>();
        app.set_runner(winit_runner);
    }

    fn name(&self) -> &str {
        "WindowPlugin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kooch_core::event::Events;
    use kooch_core::plugin::MinimalPlugins;

    #[test]
    fn window_config_default() {
        let config = WindowConfig::default();
        assert_eq!(config.title, "Kóoch");
        assert_eq!(config.width, 1280);
        assert_eq!(config.height, 720);
    }

    #[test]
    fn window_plugin_registers_events() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugin(WindowPlugin::default());

        assert!(app.resources().contains::<WindowConfig>());
        assert!(app.resources().contains::<Events<WindowResized>>());
        assert!(app
            .resources()
            .contains::<Events<WindowCloseRequested>>());
    }

    #[test]
    fn window_plugin_custom_config() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugin(WindowPlugin {
            title: "Test Game".to_string(),
            width: 1920,
            height: 1080,
        });

        let config = app.resources().get::<WindowConfig>().unwrap();
        assert_eq!(config.title, "Test Game");
        assert_eq!(config.width, 1920);
        assert_eq!(config.height, 1080);
    }
}

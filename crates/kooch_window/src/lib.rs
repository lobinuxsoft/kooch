//! `kooch_window` — a winit window driving the engine's frame loop. [`WindowPlugin`] swaps the
//! headless runner for winit's event loop.
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
//! Extra windows (an editor panel torn off onto another monitor) are asked for through
//! [`ExtraWindows`]; the frame is still driven by the main window alone.

pub mod event;
mod extra;
pub mod handle;
pub mod icon;
mod mode;
pub mod runner;
pub mod title_metrics;
mod winit_app;

pub use event::{WindowCloseRequested, WindowResized};
pub use extra::ExtraWindows;
pub use handle::WindowHandle;
pub use runner::winit_runner;

use kooch_core::app::App;
use kooch_core::plugin::Plugin;

/// Configuration for the engine window, inserted by [`WindowPlugin::build()`] and read in
/// `resumed()`; other plugins may change it first.
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

/// Plugin that creates the window and runs the engine on winit's event loop: inserts
/// [`WindowConfig`], registers the window events, sets [`winit_runner`], and publishes
/// [`WindowHandle`] once resumed.
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
    /// Whether this window follows the project's `window_mode`. 🔴 `false` in the editor, which also
    /// loads a project's `.rendersettings` and would otherwise go full screen itself.
    pub applies_window_mode: bool,
}

impl Default for WindowPlugin {
    fn default() -> Self {
        let config = WindowConfig::default();
        Self {
            title: config.title,
            width: config.width,
            height: config.height,
            applies_window_mode: true,
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

        app.insert_resource(ExtraWindows::default());
        app.add_event::<WindowResized>();
        app.add_event::<WindowCloseRequested>();

        // Runs whether or not it will write anything: the system checks
        // `FrameMetrics::report` and returns, which keeps the decision in
        // one place instead of splitting it between here and there.
        app.insert_resource(title_metrics::TitleMetricsState::default());
        app.add_system(
            kooch_core::stage::Stage::Last,
            title_metrics::title_metrics_system,
        );
        // Published in every host, the editor included: an options menu and the Game view's
        // resolution dropdown both read the list, and reading it changes nothing.
        app.add_system(
            kooch_core::stage::Stage::Last,
            mode::publish_display_modes_system,
        );
        // After `apply_render_settings_system` publishes the resource in `Update`, so a change
        // lands the same frame. Not registered in a host that does not own a game's window.
        if self.applies_window_mode {
            app.add_system(
                kooch_core::stage::Stage::Last,
                mode::apply_window_mode_system,
            );
        }

        app.set_runner(winit_runner);
    }

    fn name(&self) -> &str {
        "WindowPlugin"
    }
}

#[cfg(test)]
mod tests;

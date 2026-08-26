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
pub mod icon;
mod mode;
pub mod runner;
pub mod title_metrics;
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
    /// Whether this window follows the project's `window_mode` setting.
    ///
    /// 🔴 `false` in the editor, and that is the whole reason the field
    /// exists. The editor adds this plugin **and** the asset plugin that
    /// publishes a project's `.rendersettings`, so without it opening a
    /// project whose `window_mode` is fullscreen would put the EDITOR
    /// full screen. The setting describes the game's window, and the
    /// editor's window is not the game's.
    ///
    /// The same argument the editor already makes for `FrameMetrics`:
    /// the engine's own reporting is for a game, and a tool that shows
    /// it is showing the wrong thing.
    ///
    /// ⚠️ `vsync` needs no equivalent only because
    /// `apply_presentation_system` lives in `RenderPlugin`, which the
    /// editor must not add — see `bootstrap.rs`. That is an accident of
    /// where it was registered, not a rule, and moving it would
    /// reintroduce the same bug.
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
        // After `apply_render_settings_system`, which publishes the
        // resource in `Update`, so a change lands on the frame it is
        // made. The system does nothing until something asks for a mode.
        //
        // Not registered at all in a host that does not own a game's
        // window, rather than registered and skipped: there is nothing
        // per-frame to decide, and a system that is never right to run
        // should not be in the schedule.
        if self.applies_window_mode {
            // The list is published whether or not anything asks for a
            // mode: a game's options menu needs it to draw the dropdown
            // at all, and it costs one enumeration on the first frame
            // the window exists.
            app.add_system(
                kooch_core::stage::Stage::Last,
                mode::publish_display_modes_system,
            );
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

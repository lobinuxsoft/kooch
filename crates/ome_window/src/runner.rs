//! Winit-based game loop runner.
//!
//! Replaces the default headless runner with one that integrates into
//! winit's event loop. Frame ticks are driven by `RedrawRequested` events,
//! staying synchronized with the compositor.

use winit::event_loop::EventLoop;

use ome_core::app::App;

use crate::winit_app::WinitApp;

/// Runs the engine inside a winit event loop.
///
/// This function satisfies the [`Runner`](ome_core::runner::Runner) signature.
/// It creates a winit `EventLoop`, wraps the `App` in a [`WinitApp`], and
/// hands control to the platform event loop.
///
/// # Panics
/// - If the event loop cannot be created.
/// - If `run_app` fails (platform-specific).
pub fn winit_runner(app: App) {
    let event_loop = EventLoop::new().expect("failed to create winit event loop");
    let mut winit_app = WinitApp::new(app);
    event_loop
        .run_app(&mut winit_app)
        .expect("winit event loop error");
}

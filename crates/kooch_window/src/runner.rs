//! Winit-based game loop runner.
//!
//! Replaces the default headless runner with one that integrates into
//! winit's event loop. Frame ticks are driven by `RedrawRequested` events,
//! staying synchronized with the compositor.

use winit::event_loop::EventLoop;

use kooch_core::app::App;
use kooch_core::frame_pacing::FrameWaker;

use crate::winit_app::{WakeUp, WinitApp};

/// Runs the engine inside a winit event loop.
///
/// This function satisfies the [`Runner`](kooch_core::runner::Runner) signature.
/// It creates a winit `EventLoop`, wraps the `App` in a [`WinitApp`], and
/// hands control to the platform event loop.
///
/// The loop carries a user-event channel purely so a
/// [`FrameWaker`] can interrupt a sleep from another thread; `request_redraw`
/// is not a documented cross-thread wake-up on every platform, whereas the
/// proxy is exactly that.
///
/// # Panics
/// - If the event loop cannot be created.
/// - If `run_app` fails (platform-specific).
pub fn winit_runner(app: App) {
    let event_loop = EventLoop::<WakeUp>::with_user_event()
        .build()
        .expect("failed to create winit event loop");

    if let Some(waker) = app.resources.get::<FrameWaker>().cloned() {
        let proxy = event_loop.create_proxy();
        // A send error means the loop has already exited; the wake had
        // nowhere to go and dropping it is the correct outcome.
        waker.set_notify(move || {
            let _ = proxy.send_event(WakeUp);
        });
    }

    let mut winit_app = WinitApp::new(app);
    event_loop
        .run_app(&mut winit_app)
        .expect("winit event loop error");
}

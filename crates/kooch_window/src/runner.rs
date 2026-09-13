//! Winit-based runner: frames are driven by `RedrawRequested`, in step with the compositor.

use winit::event_loop::EventLoop;

use kooch_core::app::App;
use kooch_core::frame_pacing::FrameWaker;

use crate::winit_app::{WakeUp, WinitApp};

/// Runs the engine inside a winit event loop. The user-event channel lets a [`FrameWaker`] wake a
/// sleep from another thread, which `request_redraw` does not do on every platform.
/// Panics if the event loop cannot be created or `run_app` fails.
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

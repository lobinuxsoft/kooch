//! Internal winit application handler.
//!
//! Implements `ApplicationHandler` to bridge winit's event loop with the
//! engine's game loop. Frame ticks happen on `RedrawRequested`, synchronized
//! with the compositor (critical for Wayland).
//!
//! # Why the loop is allowed to stop
//!
//! It used to ask for the next redraw at the end of every frame, no matter
//! what, so the loop fed itself forever and an idle process still pinned a
//! core (#656). Now each frame states what the next one needs through
//! [`FrameRequest`], and this handler turns that into a `ControlFlow`. An
//! app that never inserts the resource keeps spinning — which is right for
//! a game, and wrong for an editor showing a still image.

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::{Window, WindowAttributes, WindowId};

use ome_core::app::App;
use ome_core::event::{AppExit, Events};
use ome_core::frame_pacing::{FramePace, FrameRequest, FrameWaker};
use ome_core::gpu::GpuContext;
use ome_core::raw_event::RawEventHandler;
use ome_core::time::Time;

use crate::WindowConfig;
use crate::event::{WindowCloseRequested, WindowResized};
use crate::handle::WindowHandle;

/// The user event a [`FrameWaker`] sends to break the loop out of a sleep.
///
/// It carries nothing: arriving *is* the message.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WakeUp;

/// Bridges winit's event loop with the engine's frame-based game loop.
///
/// Owns the `App` and drives the engine tick from `RedrawRequested` events.
pub(crate) struct WinitApp {
    app: App,
    window: Option<Arc<Window>>,
    startup_complete: bool,
    /// Cloned out of `Resources` up front so the sleep decision is a
    /// field read rather than a resource lookup on every frame.
    waker: Option<FrameWaker>,
}

impl WinitApp {
    /// Creates a new `WinitApp` wrapping the given engine app.
    pub(crate) fn new(app: App) -> Self {
        let waker = app.resources.get::<FrameWaker>().cloned();
        Self {
            app,
            window: None,
            startup_complete: false,
            waker,
        }
    }

    /// Asks for one more frame, if there is a window to ask.
    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Turns this frame's [`FrameRequest`] into the next `ControlFlow`.
    ///
    /// A missing resource means the app never opted into idling, so it
    /// keeps the pre-#656 behaviour: poll and redraw, always.
    fn schedule_next_frame(&mut self, event_loop: &ActiveEventLoop) {
        let mut pace = self
            .app
            .resources
            .get_mut::<FrameRequest>()
            .map(FrameRequest::take)
            .unwrap_or(FramePace::Continuous);

        // A wake that landed *during* the frame we just ran would
        // otherwise be slept straight through: the requester did its
        // store while we were busy, and we are only now deciding to
        // stop. Clearing the flag here — after the frame, before the
        // sleep — is what closes that window.
        if self.waker.as_ref().is_some_and(FrameWaker::take_pending) {
            pace = FramePace::Continuous;
        }

        match pace {
            FramePace::Continuous => {
                event_loop.set_control_flow(ControlFlow::Poll);
                self.request_redraw();
            }
            // A deadline far enough out to overflow the clock is a
            // deadline that never arrives; sleep rather than panic on
            // the addition.
            FramePace::After(delay) => match Instant::now().checked_add(delay) {
                Some(deadline) => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
                None => event_loop.set_control_flow(ControlFlow::Wait),
            },
            FramePace::Wait => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    /// Runs a single engine frame tick (mirrors `default_runner` logic).
    fn tick_frame(&mut self) {
        self.update_events();

        if self.should_exit() {
            tracing::info!("AppExit received, shutting down");
            return;
        }

        let fixed_steps = {
            let time = self
                .app
                .resources
                .get_mut::<Time>()
                .expect("Time resource not found");
            time.update()
        };

        self.app.schedule.run_pre_physics(&mut self.app.resources);

        for _ in 0..fixed_steps {
            self.app.schedule.run_fixed_stages(&mut self.app.resources);
        }

        self.app.schedule.run_post_physics(&mut self.app.resources);
    }

    /// Swaps double buffers for all registered event types.
    ///
    /// It used to name three of them under this same comment, which meant
    /// every event any plugin added was written and never became readable —
    /// in the editor as much as anywhere, since this is the runner a
    /// windowed app uses. Asking beats remembering.
    fn update_events(&mut self) {
        ome_core::event::update_all_events(&mut self.app.resources);
    }

    /// Returns `true` if an `AppExit` event has been sent.
    fn should_exit(&self) -> bool {
        self.app
            .resources
            .get::<Events<AppExit>>()
            .is_some_and(|events| !events.is_empty())
    }
}

impl ApplicationHandler<WakeUp> for WinitApp {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        // A `WaitUntil` deadline expiring is the only wake-up nothing
        // else reports: no window event arrives, so without this the
        // frame the deadline was set for never happens.
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            self.request_redraw();
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: WakeUp) {
        // Some other thread wants a frame — the remote server's listener,
        // parked on a reply only the main loop can produce.
        self.request_redraw();
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let config = self
            .app
            .resources
            .get::<WindowConfig>()
            .expect("WindowConfig resource not found");

        let attrs = WindowAttributes::default()
            .with_title(&config.title)
            .with_inner_size(winit::dpi::LogicalSize::new(config.width, config.height));

        let window = event_loop
            .create_window(attrs)
            .expect("failed to create window");

        let window = Arc::new(window);

        tracing::info!(
            title = config.title,
            width = config.width,
            height = config.height,
            "Window created"
        );

        self.app
            .resources
            .insert(WindowHandle::new(Arc::clone(&window)));

        let size = window.inner_size();
        match GpuContext::new(Arc::clone(&window), size.width, size.height) {
            Ok(gpu) => {
                self.app.resources.insert(gpu);
            }
            Err(e) => {
                tracing::error!("Failed to initialize GPU: {e}");
                event_loop.exit();
                return;
            }
        }

        if !self.startup_complete {
            self.app.schedule.run_startup(&mut self.app.resources);
            self.startup_complete = true;
        }

        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        // Forward events to registered handler (e.g., egui overlay).
        if let Some(window) = self.window.clone() {
            if let Some(handler) = self.app.resources.get_mut::<Box<dyn RawEventHandler>>() {
                handler.on_event(&*window, &event);
            }
        }

        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("Window close requested");

                if let Some(events) = self.app.resources.get_mut::<Events<WindowCloseRequested>>() {
                    events.send(WindowCloseRequested);
                }
                if let Some(events) = self.app.resources.get_mut::<Events<AppExit>>() {
                    events.send(AppExit);
                }

                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.app.resources.get_mut::<GpuContext>() {
                    gpu.resize(size.width, size.height);
                }
                if let Some(events) = self.app.resources.get_mut::<Events<WindowResized>>() {
                    events.send(WindowResized {
                        width: size.width,
                        height: size.height,
                    });
                }
                tracing::debug!(width = size.width, height = size.height, "Window resized");
                self.request_redraw();
            }

            WindowEvent::RedrawRequested => {
                self.tick_frame();

                if self.should_exit() {
                    event_loop.exit();
                    return;
                }

                self.schedule_next_frame(event_loop);
            }

            // Everything else — a moved cursor, a key, a scroll, a focus
            // change — is an input the UI has not drawn yet. While the
            // loop is idle it is also the *only* thing that will produce
            // a frame, so it asks for one rather than assuming the next
            // one is already on its way.
            _ => self.request_redraw(),
        }
    }
}

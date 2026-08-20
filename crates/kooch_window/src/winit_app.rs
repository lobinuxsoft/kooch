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

use kooch_core::app::App;
use kooch_core::event::{AppExit, Events};
use kooch_core::frame_pacing::{FramePace, FrameRequest, FrameWaker};
use kooch_core::gpu::GpuContext;
use kooch_core::raw_event::RawEventHandlers;
use kooch_core::time::Time;

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
    /// Last pace logged, so the decision is reported on change rather
    /// than sixty times a second.
    last_pace: Option<FramePace>,
    /// Name of the last window event that asked for a redraw, for the
    /// same log line — "who woke it" is the question that matters when
    /// an idle editor refuses to idle.
    last_waker_event: &'static str,
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
            last_pace: None,
            last_waker_event: "none",
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

        if self.last_pace != Some(pace) {
            tracing::debug!(?pace, woken_by = self.last_waker_event, "frame pacing");
            self.last_pace = Some(pace);
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
        kooch_core::event::update_all_events(&mut self.app.resources);
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

        // 🔴 Only the ENVIRONMENT override is applied here, not the
        // project's setting. The settings asset needs the asset server,
        // which needs the GPU, which needs this window — so the asset's
        // mode cannot exist yet and lands a few frames later through
        // `mode::apply_window_mode_system`. The variable can be read
        // before anything, and a measurement run asking for fullscreen
        // should not spend its first frames in a window.
        let mode = kooch_core::window_mode::mode_override();
        let attrs = WindowAttributes::default()
            .with_title(&config.title)
            .with_window_icon(crate::icon::window_icon())
            .with_inner_size(winit::dpi::LogicalSize::new(config.width, config.height))
            .with_decorations(mode.is_none_or(|mode| mode.decorated()))
            .with_fullscreen(
                mode.filter(|mode| mode.fullscreen())
                    .map(|_| winit::window::Fullscreen::Borderless(None)),
            );

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
                // #536 — the DLSS handles as their own resource. The
                // render systems remove `GpuContext` for the length of a
                // frame, so a pass that reached for the adapter through
                // it mid-frame would find nothing.
                self.app.resources.insert(gpu.dlss_runtime());
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
        // Forward events to registered handlers (e.g., egui overlay,
        // gameplay input), in order, until one consumes the event.
        if let Some(window) = self.window.clone() {
            if let Some(handlers) = self.app.resources.get_mut::<RawEventHandlers>() {
                handlers.dispatch(&*window, &event);
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

            // An input the UI has not drawn yet. While the loop is idle
            // this is also the *only* thing that will produce a frame, so
            // it asks for one rather than assuming the next is on its way.
            //
            // By list rather than by catch-all, because two of the events
            // winit sends most often change nothing that is drawn, and
            // asking for a frame on each of them cost more than every
            // real input put together — measured over five seconds of
            // ordinary mouse movement: 966 `AxisMotion` and 212 `Moved`
            // against 486 `CursorMoved`.
            other if wants_a_frame(&other) => {
                self.last_waker_event = window_event_name(&other);
                self.request_redraw();
            }

            _ => {}
        }
    }
}

/// A stable name for a window event, for the pacing log.
///
/// `Debug` would print the payload, and a cursor position changing every
/// line is the opposite of what this is read for.
fn window_event_name(event: &WindowEvent) -> &'static str {
    match event {
        WindowEvent::CursorMoved { .. } => "CursorMoved",
        WindowEvent::CursorEntered { .. } => "CursorEntered",
        WindowEvent::CursorLeft { .. } => "CursorLeft",
        WindowEvent::MouseInput { .. } => "MouseInput",
        WindowEvent::MouseWheel { .. } => "MouseWheel",
        WindowEvent::KeyboardInput { .. } => "KeyboardInput",
        WindowEvent::ModifiersChanged(_) => "ModifiersChanged",
        WindowEvent::Focused(_) => "Focused",
        WindowEvent::Moved(_) => "Moved",
        WindowEvent::Occluded(_) => "Occluded",
        WindowEvent::AxisMotion { .. } => "AxisMotion",
        WindowEvent::ScaleFactorChanged { .. } => "ScaleFactorChanged",
        WindowEvent::Ime(_) => "Ime",
        WindowEvent::TouchpadPressure { .. } => "TouchpadPressure",
        WindowEvent::PinchGesture { .. } => "PinchGesture",
        WindowEvent::PanGesture { .. } => "PanGesture",
        WindowEvent::DoubleTapGesture { .. } => "DoubleTapGesture",
        WindowEvent::RotationGesture { .. } => "RotationGesture",
        WindowEvent::ThemeChanged(_) => "ThemeChanged",
        WindowEvent::HoveredFile(_) => "HoveredFile",
        WindowEvent::HoveredFileCancelled => "HoveredFileCancelled",
        WindowEvent::DroppedFile(_) => "DroppedFile",
        WindowEvent::ActivationTokenDone { .. } => "ActivationTokenDone",
        WindowEvent::Destroyed => "Destroyed",
        WindowEvent::Touch(_) => "Touch",
        _ => "other",
    }
}

/// Whether an event changes something the next frame would draw.
///
/// The two exclusions are the point:
///
/// - **`AxisMotion`** duplicates `CursorMoved`. winit reports the raw
///   device axes *as well as* the resulting cursor position, at roughly
///   twice the rate, and egui reads the cursor. Redrawing for both draws
///   the same frame twice.
/// - **`Moved`** is the window changing position on the desktop. Not one
///   pixel of its contents differs, and a compositor emits it for every
///   step of a drag.
///
/// Everything unmatched is excluded too. A new winit event is far more
/// likely to be bookkeeping than something the UI must react to, and the
/// failure mode of a miss is one late repaint — against a catch-all,
/// whose failure mode is the loop never sleeping again.
fn wants_a_frame(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::KeyboardInput { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::Ime(_)
            | WindowEvent::Focused(_)
            | WindowEvent::Occluded(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::ThemeChanged(_)
            | WindowEvent::HoveredFile(_)
            | WindowEvent::HoveredFileCancelled
            | WindowEvent::DroppedFile(_)
            | WindowEvent::Touch(_)
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::PinchGesture { .. }
            | WindowEvent::PanGesture { .. }
            | WindowEvent::DoubleTapGesture { .. }
            | WindowEvent::RotationGesture { .. }
    )
}

#[cfg(test)]
mod tests;

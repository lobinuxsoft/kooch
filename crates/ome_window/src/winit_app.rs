//! Internal winit application handler.
//!
//! Implements `ApplicationHandler` to bridge winit's event loop with the engine's
//! game loop. Frame ticks happen on `RedrawRequested`, synchronized with the
//! compositor (critical for Wayland).

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes, WindowId};

use ome_core::app::App;
use ome_core::event::{AppExit, Events};
use ome_core::gpu::GpuContext;
use ome_core::raw_event::RawEventHandler;
use ome_core::time::Time;

use crate::event::{WindowCloseRequested, WindowResized};
use crate::handle::WindowHandle;
use crate::WindowConfig;

/// Bridges winit's event loop with the engine's frame-based game loop.
///
/// Owns the `App` and drives the engine tick from `RedrawRequested` events.
pub(crate) struct WinitApp {
    app: App,
    window: Option<Arc<Window>>,
    startup_complete: bool,
}

impl WinitApp {
    /// Creates a new `WinitApp` wrapping the given engine app.
    pub(crate) fn new(app: App) -> Self {
        Self {
            app,
            window: None,
            startup_complete: false,
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
            self.app
                .schedule
                .run_fixed_stages(&mut self.app.resources);
        }

        self.app
            .schedule
            .run_post_physics(&mut self.app.resources);
    }

    /// Swaps double buffers for all registered event types.
    fn update_events(&mut self) {
        if let Some(events) = self.app.resources.get_mut::<Events<AppExit>>() {
            events.update();
        }
        if let Some(events) = self.app.resources.get_mut::<Events<WindowResized>>() {
            events.update();
        }
        if let Some(events) = self.app.resources.get_mut::<Events<WindowCloseRequested>>() {
            events.update();
        }
    }

    /// Returns `true` if an `AppExit` event has been sent.
    fn should_exit(&self) -> bool {
        self.app
            .resources
            .get::<Events<AppExit>>()
            .is_some_and(|events| !events.is_empty())
    }
}

impl ApplicationHandler for WinitApp {
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
            if let Some(handler) = self
                .app
                .resources
                .get_mut::<Box<dyn RawEventHandler>>()
            {
                handler.on_event(&*window, &event);
            }
        }

        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("Window close requested");

                if let Some(events) =
                    self.app.resources.get_mut::<Events<WindowCloseRequested>>()
                {
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
            }

            WindowEvent::RedrawRequested => {
                self.tick_frame();

                if self.should_exit() {
                    event_loop.exit();
                    return;
                }

                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            _ => {}
        }
    }
}

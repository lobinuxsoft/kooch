//! [`InputPlugin`] — connects the backend to a running app. A keypress's path:
//! ```text
//! winit  ──WindowEvent──▶  RawEventHandlers  ──▶  WinitEventCollector
//!                                                        │ queues
//!                                                        ▼
//!   Stage::Input:  begin_frame()  →  feed each queued event  →  poll()
//!                                                        │
//!   Stage::Update: gameplay reads is_pressed / just_pressed
//! ```
//!
//!
//! Queued, not applied on arrival: winit delivers between frames, and queueing keeps clear → apply
//! → read inside one frame, where `just_pressed` lives.

use std::sync::{Arc, Mutex};

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::raw_event::{RawEventHandler, RawEventHandlers};
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;
use winit::event::WindowEvent;

use crate::backend::{InputBackend, InputEvent};
use crate::winit_gilrs_backend::WinitGilrsBackend;

/// Window events waiting for the backend this frame, shared between the runner's raw-event handler
/// and a system.
#[derive(Clone, Default)]
pub struct PendingWindowEvents(Arc<Mutex<Vec<WindowEvent>>>);

impl PendingWindowEvents {
    fn push(&self, event: WindowEvent) {
        self.0
            .lock()
            .expect("pending input mutex poisoned")
            .push(event);
    }

    fn take(&self) -> Vec<WindowEvent> {
        std::mem::take(&mut *self.0.lock().expect("pending input mutex poisoned"))
    }
}

/// Raw mouse motion waiting for the backend this frame, summed as it arrives: the device reports far
/// more often than a frame, and the backend only needs the total (#1266).
#[derive(Clone, Default)]
pub struct PendingMotion(Arc<Mutex<glam::Vec2>>);

impl PendingMotion {
    fn add(&self, delta: glam::Vec2) {
        *self.0.lock().expect("pending motion mutex poisoned") += delta;
    }

    fn take(&self) -> glam::Vec2 {
        std::mem::take(&mut *self.0.lock().expect("pending motion mutex poisoned"))
    }
}

/// Queues the window events the backend cares about, never consuming them — focused UI gets the key
/// first.
struct WinitEventCollector {
    pending: PendingWindowEvents,
    motion: PendingMotion,
}

impl RawEventHandler for WinitEventCollector {
    fn on_event(&mut self, _window: &dyn std::any::Any, event: &dyn std::any::Any) -> bool {
        // The device's own motion, which a window event cannot carry: two cursor positions stop
        // at the window's edge, and a captured cursor does not move at all.
        if let Some(winit::event::DeviceEvent::MouseMotion { delta }) =
            event.downcast_ref::<winit::event::DeviceEvent>()
        {
            self.motion
                .add(glam::Vec2::new(delta.0 as f32, delta.1 as f32));
            return false;
        }
        let Some(event) = event.downcast_ref::<WindowEvent>() else {
            return false;
        };
        // By list, not by catch-all. The runner forwards every window
        // event, and cloning the ones the backend discards would be a
        // heap allocation per mouse motion for nothing.
        if matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::CursorMoved { .. }
        ) {
            self.pending.push(event.clone());
        }
        false
    }
}

/// Inserts an input backend and drives its frame cycle; gameplay then reads `Box<dyn
/// InputBackend>`:
///
/// ```ignore
/// fn move_player(resources: &mut Resources) {
///     let Some(input) = resources.get::<Box<dyn InputBackend>>() else { return };
///     if input.is_pressed(KeyCode::KeyW) { /* … */ }
/// }
/// ```
#[derive(Default)]
pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        let pending = PendingWindowEvents::default();
        let motion = PendingMotion::default();

        let backend: Box<dyn InputBackend> = Box::new(WinitGilrsBackend::new());
        app.insert_resource(pending.clone())
            .insert_resource(motion.clone())
            .insert_resource(backend)
            .add_event::<InputEvent>()
            .add_system(Stage::Input, pump_input)
            // Last, after every system that might ask, so a request lands the frame it is made.
            .add_system(Stage::Last, crate::cursor::apply_cursor_system);

        // Registered from a Startup system so plugin add order decides — letting the editor's egui
        // take keys for a focused field first.
        app.add_system(Stage::Startup, move |resources: &mut Resources| {
            let collector: Box<dyn RawEventHandler> = Box::new(WinitEventCollector {
                pending: pending.clone(),
                motion: motion.clone(),
            });
            resources
                .get_or_default::<RawEventHandlers>()
                .push(collector);
        });
    }

    fn name(&self) -> &str {
        "InputPlugin"
    }
}

/// Advances the backend one frame — forget edges, apply queued events, drain devices — in
/// [`Stage::Input`], before `PreUpdate`.
fn pump_input(resources: &mut Resources) {
    // Cloned out first: the queue and the backend are two resources, and
    // holding a borrow of one rules out asking for the other.
    let Some(pending) = resources.get::<PendingWindowEvents>().cloned() else {
        return;
    };
    let queued = pending.take();
    let motion = resources
        .get::<PendingMotion>()
        .map(PendingMotion::take)
        .unwrap_or_default();

    let Some(backend) = resources.get_mut::<Box<dyn InputBackend>>() else {
        return;
    };

    backend.begin_frame();
    for event in &queued {
        backend.feed_window_event(event);
    }
    backend.feed_mouse_motion(motion);
    let events = backend.poll();

    if let Some(buffer) = resources.get_mut::<kooch_core::event::Events<InputEvent>>() {
        for event in events {
            buffer.send(event);
        }
    }
}

#[cfg(test)]
mod tests;

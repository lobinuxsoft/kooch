//! [`InputPlugin`] — what makes the rest of this crate reachable.
//!
//! Everything else here compiled for months without a single call site
//! outside the crate: no backend was ever constructed, no resource was
//! ever inserted, and winit's `KeyboardInput` events reached the window
//! runner and asked for a redraw. A game could not read a key.
//!
//! # The path an keypress takes
//!
//! ```text
//! winit  ──WindowEvent──▶  RawEventHandlers  ──▶  WinitEventCollector
//!                                                        │ queues
//!                                                        ▼
//!   Stage::Input:  begin_frame()  →  feed each queued event  →  poll()
//!                                                        │
//!   Stage::Update: gameplay reads is_pressed / just_pressed
//! ```
//!
//! # Why the events are queued instead of applied on arrival
//!
//! winit delivers events *between* frames, and `just_pressed` is true
//! for exactly one frame. Applying on arrival means the edge is recorded
//! against no particular frame, and whichever frame boundary clears it
//! decides whether anyone ever saw it. Queuing moves the whole sequence
//! — clear, apply, read — inside one frame, where the order is ours.

use std::sync::{Arc, Mutex};

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::raw_event::{RawEventHandler, RawEventHandlers};
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;
use winit::event::WindowEvent;

use crate::backend::{InputBackend, InputEvent};
use crate::winit_gilrs_backend::WinitGilrsBackend;

/// Window events waiting to be applied to the backend this frame.
///
/// Shared because the producer is a raw-event handler owned by the
/// window runner and the consumer is a system holding `Resources`.
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

/// Queues the window events the input backend cares about.
///
/// Never consumes: input is the last thing that wants a key, after any
/// UI that had it focused.
struct WinitEventCollector {
    pending: PendingWindowEvents,
}

impl RawEventHandler for WinitEventCollector {
    fn on_event(&mut self, _window: &dyn std::any::Any, event: &dyn std::any::Any) -> bool {
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

/// Inserts an input backend and drives its frame cycle.
///
/// After this plugin, `Box<dyn InputBackend>` is a resource any gameplay
/// system can read:
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

        let backend: Box<dyn InputBackend> = Box::new(WinitGilrsBackend::new());
        app.insert_resource(pending.clone())
            .insert_resource(backend)
            .add_event::<InputEvent>()
            .add_system(Stage::Input, pump_input);

        // Registered from a Startup system rather than here, so the order
        // is the order plugins were added rather than the order they were
        // built. That is what lets an editor's egui overlay register
        // first and keep a keystroke aimed at a focused text field.
        app.add_system(Stage::Startup, move |resources: &mut Resources| {
            let collector: Box<dyn RawEventHandler> = Box::new(WinitEventCollector {
                pending: pending.clone(),
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

/// Advances the backend one frame: forget last frame's edges, apply the
/// events that arrived since, drain the device sources.
///
/// Runs in [`Stage::Input`], which sits before `PreUpdate` — so a
/// gameplay system reads the key that was pressed on *this* frame.
fn pump_input(resources: &mut Resources) {
    // Cloned out first: the queue and the backend are two resources, and
    // holding a borrow of one rules out asking for the other.
    let Some(pending) = resources.get::<PendingWindowEvents>().cloned() else {
        return;
    };
    let queued = pending.take();

    let Some(backend) = resources.get_mut::<Box<dyn InputBackend>>() else {
        return;
    };

    backend.begin_frame();
    for event in &queued {
        backend.feed_window_event(event);
    }
    let events = backend.poll();

    if let Some(buffer) = resources.get_mut::<kooch_core::event::Events<InputEvent>>() {
        for event in events {
            buffer.send(event);
        }
    }
}

#[cfg(test)]
mod tests;

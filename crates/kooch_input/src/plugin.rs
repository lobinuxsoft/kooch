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

        let collector: Box<dyn RawEventHandler> = Box::new(WinitEventCollector {
            pending: pending.clone(),
        });
        app.resources_mut()
            .get_or_default::<RawEventHandlers>()
            .push(collector);

        let backend: Box<dyn InputBackend> = Box::new(WinitGilrsBackend::new());
        app.insert_resource(pending)
            .insert_resource(backend)
            .add_event::<InputEvent>()
            .add_system(Stage::Input, pump_input);
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
mod tests {
    use super::*;
    use crate::backend::{GamepadAxis, GamepadButton, GamepadId, KeyCode, MouseButton};
    use crate::mock_backend::MockInputBackend;
    use glam::Vec2;
    use std::collections::HashSet;

    /// What the backend was asked to do, in order.
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    enum Call {
        BeginFrame,
        FeedWindowEvent,
        Poll,
    }

    /// A backend that records its frame cycle and nothing else.
    ///
    /// The order is the whole point: clearing after the frame's events
    /// are applied wipes them, and that is the bug this pins. A test
    /// against real state would pass either way as long as the backend
    /// happened to be empty.
    #[derive(Default)]
    struct RecordingBackend {
        calls: Arc<Mutex<Vec<Call>>>,
    }

    impl RecordingBackend {
        fn record(&self, call: Call) {
            self.calls.lock().unwrap().push(call);
        }
    }

    impl InputBackend for RecordingBackend {
        fn begin_frame(&mut self) {
            self.record(Call::BeginFrame);
        }
        fn feed_window_event(&mut self, _event: &WindowEvent) {
            self.record(Call::FeedWindowEvent);
        }
        fn poll(&mut self) -> Vec<InputEvent> {
            self.record(Call::Poll);
            Vec::new()
        }
        fn is_pressed(&self, _key: KeyCode) -> bool {
            false
        }
        fn just_pressed(&self, _key: KeyCode) -> bool {
            false
        }
        fn just_released(&self, _key: KeyCode) -> bool {
            false
        }
        fn pressed_keys(&self) -> HashSet<KeyCode> {
            HashSet::new()
        }
        fn is_mouse_pressed(&self, _button: MouseButton) -> bool {
            false
        }
        fn mouse_position(&self) -> Vec2 {
            Vec2::ZERO
        }
        fn mouse_delta(&self) -> Vec2 {
            Vec2::ZERO
        }
        fn gamepads(&self) -> Vec<GamepadId> {
            Vec::new()
        }
        fn is_button_pressed(&self, _gamepad: GamepadId, _button: GamepadButton) -> bool {
            false
        }
        fn axis_value(&self, _gamepad: GamepadId, _axis: GamepadAxis) -> f32 {
            0.0
        }
    }

    /// A window event whose kind the collector accepts, built without a
    /// window: `Destroyed` carries nothing and is trivially constructible.
    /// The collector's filter rejects it, so tests that need the *queue*
    /// populated push straight into `PendingWindowEvents`.
    fn a_window_event() -> WindowEvent {
        WindowEvent::Destroyed
    }

    fn resources_with(backend: Box<dyn InputBackend>) -> (Resources, PendingWindowEvents) {
        let mut resources = Resources::new();
        let pending = PendingWindowEvents::default();
        resources.insert(pending.clone());
        resources.insert(backend);
        (resources, pending)
    }

    #[test]
    fn the_frame_clears_edges_before_applying_the_events_that_arrived() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = RecordingBackend {
            calls: Arc::clone(&calls),
        };
        let (mut resources, pending) = resources_with(Box::new(backend));
        pending.push(a_window_event());
        pending.push(a_window_event());

        pump_input(&mut resources);

        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                Call::BeginFrame,
                Call::FeedWindowEvent,
                Call::FeedWindowEvent,
                Call::Poll,
            ],
            "an edge cleared after the frame's events are applied is an edge nobody reads"
        );
    }

    #[test]
    fn the_queue_is_emptied_so_an_event_is_never_applied_twice() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = RecordingBackend {
            calls: Arc::clone(&calls),
        };
        let (mut resources, pending) = resources_with(Box::new(backend));
        pending.push(a_window_event());

        pump_input(&mut resources);
        calls.lock().unwrap().clear();
        pump_input(&mut resources);

        assert_eq!(
            *calls.lock().unwrap(),
            vec![Call::BeginFrame, Call::Poll],
            "the second frame replayed the first frame's events"
        );
    }

    #[test]
    fn a_backend_forgets_last_frames_edge_but_not_the_key_still_held() {
        let mut backend = MockInputBackend::new();
        backend.press_key(KeyCode::Space);
        assert!(backend.just_pressed(KeyCode::Space));

        backend.begin_frame();

        assert!(
            !backend.just_pressed(KeyCode::Space),
            "the edge survived the frame boundary"
        );
        assert!(
            backend.is_pressed(KeyCode::Space),
            "begin_frame released a key that is still held"
        );
    }

    #[test]
    fn pumping_without_a_backend_is_a_no_op_rather_than_a_panic() {
        let mut resources = Resources::new();
        resources.insert(PendingWindowEvents::default());
        pump_input(&mut resources);
    }
}

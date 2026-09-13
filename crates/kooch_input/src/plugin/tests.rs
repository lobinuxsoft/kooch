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

/// A backend recording its frame cycle only: the order is what this pins, and real state would pass
/// while empty.
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
    fn just_button_pressed(&self, _gamepad: GamepadId, _button: GamepadButton) -> bool {
        false
    }
    fn just_button_released(&self, _gamepad: GamepadId, _button: GamepadButton) -> bool {
        false
    }
    fn axis_value(&self, _gamepad: GamepadId, _axis: GamepadAxis) -> f32 {
        0.0
    }
}

/// A window event the collector's filter rejects (`Destroyed`), so tests that need a populated
/// queue push to `PendingWindowEvents` directly.
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

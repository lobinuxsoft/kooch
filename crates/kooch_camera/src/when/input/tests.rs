//! The authored action reaching a [`CameraWhen`] (#1352).

use super::*;
use kooch_ecs::allocator::EntityAllocator;
use kooch_input::actions::{Action, Binding, ControlPath, ControlType, Role};
use kooch_input::{GamepadButton, GamepadId, MockInputBackend};

const PAD: GamepadId = GamepadId(0);
const SHOULDER: [u8; 16] = [11; 16];

/// A vcam with a condition, bound to a `button` action over `LT` or to nothing at all.
fn world(bound: bool) -> (Resources, Entity) {
    let mut resources = Resources::new();
    let mut allocator = EntityAllocator::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<CameraWhen>();
    registry.register_cpu_reflected::<WhenInput>();
    let vcam = allocator.spawn();
    registry
        .get_cpu_mut::<CameraWhen>()
        .expect("registered")
        .insert(vcam, CameraWhen::default());
    let guid = Guid::from_bytes(SHOULDER);
    registry
        .get_cpu_mut::<WhenInput>()
        .expect("registered")
        .insert(
            vcam,
            WhenInput {
                action: bound.then_some(guid),
            },
        );
    let mut loaded = LoadedActions::default();
    loaded.load(guid, shoulder());
    resources.insert(loaded);
    resources.insert(allocator);
    resources.insert(registry);
    hold(&mut resources, false);
    (resources, vcam)
}

/// A `button` over the left trigger's click, as `Shoulder.inputaction` authors it.
fn shoulder() -> Action {
    Action::new("shoulder", ControlType::Button).bind(Binding {
        role: Role::Whole(ControlPath::Button(GamepadButton::LeftTrigger)),
        processors: Vec::new(),
    })
}

/// Replaces the backend with one holding the trigger where asked.
fn hold(resources: &mut Resources, down: bool) {
    let mut backend = MockInputBackend::new();
    backend.add_gamepad(PAD);
    if down {
        backend.press_gamepad_button(PAD, GamepadButton::LeftTrigger);
    }
    resources.insert(Box::new(backend) as Box<dyn InputBackend>);
}

fn asked_of(resources: &Resources, vcam: Entity) -> bool {
    resources
        .get::<ComponentRegistry>()
        .expect("registered")
        .get_cpu::<CameraWhen>()
        .expect("registered")
        .get(vcam)
        .expect("spawned")
        .asked
}

/// The hold reaches the condition, and letting go reaches it too.
#[test]
fn a_bound_action_asks_for_the_camera() {
    let (mut resources, vcam) = world(true);
    hold(&mut resources, true);
    read_when_input(&mut resources);
    assert!(asked_of(&resources, vcam), "the trigger never arrived");

    hold(&mut resources, false);
    read_when_input(&mut resources);
    assert!(!asked_of(&resources, vcam), "letting go kept asking");
}

/// Without a binding the condition is somebody else's to write — a state machine's, a zone's.
#[test]
fn an_unbound_condition_is_untouched() {
    let (mut resources, vcam) = world(false);
    hold(&mut resources, true);
    read_when_input(&mut resources);
    assert!(!asked_of(&resources, vcam));
}

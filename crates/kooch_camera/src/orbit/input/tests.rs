//! The authored action reaching a [`CameraOrbit`] (#1258).

use super::*;
use glam::Vec2;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::entity::Entity;
use kooch_input::actions::{
    Action, Binding, Composite, ControlPath, ControlType, PartName, Role, VectorMode,
};
use kooch_input::{GamepadAxis, GamepadButton, GamepadId, MockInputBackend};

const PAD: GamepadId = GamepadId(0);
const LOOK: [u8; 16] = [7; 16];
const RECENTRE: [u8; 16] = [9; 16];

/// A vcam with an orbit, bound to a `vector2` action over the right stick or to nothing at all.
fn world(bound: bool, stick: Vec2) -> (Resources, Entity) {
    let mut resources = Resources::new();
    let mut allocator = EntityAllocator::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<CameraOrbit>();
    registry.register_cpu_reflected::<OrbitInput>();
    let vcam = allocator.spawn();
    registry
        .get_cpu_mut::<CameraOrbit>()
        .unwrap()
        .insert(vcam, CameraOrbit::default());
    let guid = Guid::from_bytes(LOOK);
    registry.get_cpu_mut::<OrbitInput>().unwrap().insert(
        vcam,
        OrbitInput {
            look: bound.then_some(guid),
            recentre: bound.then_some(Guid::from_bytes(RECENTRE)),
        },
    );

    let mut loaded = LoadedActions::default();
    loaded.load(guid, look());
    loaded.load(Guid::from_bytes(RECENTRE), recentre());
    resources.insert(loaded);
    resources.insert(allocator);
    resources.insert(registry);
    hold(&mut resources, stick);
    (resources, vcam)
}

/// A `vector2` over the right stick, as `Look.inputaction` authors it.
fn look() -> Action {
    let part = |name, axis| Binding {
        role: Role::Part {
            name,
            path: ControlPath::Axis(axis),
        },
        processors: Vec::new(),
    };
    Action::new("look", ControlType::Vector2)
        .bind(Binding {
            role: Role::CompositeHead(Composite::Vector2 {
                mode: VectorMode::Analog,
            }),
            processors: Vec::new(),
        })
        .bind(part(PartName::Right, GamepadAxis::RightStickX))
        .bind(part(PartName::Up, GamepadAxis::RightStickY))
}

/// A `button` over the right stick's click, as `Recentre.inputaction` authors it.
fn recentre() -> Action {
    Action::new("recentre", ControlType::Button).bind(Binding {
        role: Role::Whole(ControlPath::Button(GamepadButton::RightThumb)),
        processors: Vec::new(),
    })
}

/// Replaces the backend with one holding the stick where asked — the mock is not reachable through
/// `dyn InputBackend`, and a fresh one is what letting go looks like anyway.
fn hold(resources: &mut Resources, stick: Vec2) {
    let mut backend = MockInputBackend::new();
    backend.add_gamepad(PAD);
    backend.set_axis(PAD, GamepadAxis::RightStickX, stick.x);
    backend.set_axis(PAD, GamepadAxis::RightStickY, stick.y);
    resources.insert(Box::new(backend) as Box<dyn InputBackend>);
}

/// The button reaches the orbit as a **hold**; turning it into one press is the orbit's job, so a
/// script writing `recentre_now` the same way gets the same one-per-press.
#[test]
fn the_button_reaches_the_orbit() {
    let (mut resources, vcam) = world(true, Vec2::ZERO);
    click(&mut resources, true);
    read_orbit_input(&mut resources);
    assert!(
        asked_of(&resources, vcam),
        "the press never reached the orbit"
    );

    click(&mut resources, false);
    read_orbit_input(&mut resources);
    assert!(!asked_of(&resources, vcam), "letting go asked for a return");
}

fn asked_of(resources: &Resources, vcam: Entity) -> bool {
    resources
        .get::<ComponentRegistry>()
        .unwrap()
        .get_cpu::<CameraOrbit>()
        .unwrap()
        .get(vcam)
        .unwrap()
        .recentre_now
}

/// Holds or releases the right stick's click, keeping the sticks where they were.
fn click(resources: &mut Resources, down: bool) {
    let mut backend = MockInputBackend::new();
    backend.add_gamepad(PAD);
    if down {
        backend.press_gamepad_button(PAD, GamepadButton::RightThumb);
    }
    resources.insert(Box::new(backend) as Box<dyn InputBackend>);
}

fn look_of(resources: &Resources, vcam: Entity) -> Vec2 {
    resources
        .get::<ComponentRegistry>()
        .unwrap()
        .get_cpu::<CameraOrbit>()
        .unwrap()
        .get(vcam)
        .unwrap()
        .look
}

#[test]
fn a_bound_action_fills_the_look() {
    let (mut resources, vcam) = world(true, Vec2::X);
    read_orbit_input(&mut resources);
    assert!(
        look_of(&resources, vcam).x > 0.5,
        "the stick never reached the orbit: {:?}",
        look_of(&resources, vcam)
    );
}

/// Without a binding the orbit is somebody else's to write — a cutscene's, a script's.
#[test]
fn an_unbound_orbit_is_untouched() {
    let (mut resources, vcam) = world(false, Vec2::X);
    read_orbit_input(&mut resources);
    assert_eq!(look_of(&resources, vcam), Vec2::ZERO);
}

/// 🔴 Zero is written like anything else: letting go is what starts a recentring, and skipping the
/// write would leave the camera turning on its own.
#[test]
fn letting_go_writes_zero() {
    let (mut resources, vcam) = world(true, Vec2::X);
    read_orbit_input(&mut resources);
    hold(&mut resources, Vec2::ZERO);
    read_orbit_input(&mut resources);
    assert_eq!(look_of(&resources, vcam), Vec2::ZERO);
}

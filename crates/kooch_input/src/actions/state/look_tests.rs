//! Mouse look through an action (#1266): mouse motion and a stick in one `look`, read as the same
//! kind of number.

use super::*;
use crate::actions::action::Action;
use crate::actions::binding::Binding;
use crate::actions::processor::Processor;
use crate::ids::{GamepadAxis, GamepadId, MouseAxis};
use crate::mock_backend::MockInputBackend;

/// A `look` the way a game binds it: the mouse scaled from pixels per second to a stick's unit, and
/// the right stick as it is.
fn look() -> Action {
    Action::new("look", ControlType::Vector2).bind_all([
        Binding::composite(Composite::Vector2 {
            mode: VectorMode::Analog,
        })
        .with(Processor::Scale { factor: 0.001 }),
        Binding::part(PartName::Right, ControlPath::MouseMotion(MouseAxis::X)),
        Binding::part(PartName::Up, ControlPath::MouseMotion(MouseAxis::Y)),
        Binding::composite(Composite::Vector2 {
            mode: VectorMode::Analog,
        }),
        Binding::part(PartName::Right, ControlPath::Axis(GamepadAxis::RightStickX)),
        Binding::part(PartName::Up, ControlPath::Axis(GamepadAxis::RightStickY)),
    ])
}

/// 🔴 The mouse is a velocity: a thousand pixels a second to the right reads as a full stick to the
/// right, whatever the frame rate — a per-frame delta would halve at twice the frames.
#[test]
fn mouse_motion_reads_as_velocity() {
    let mut backend = MockInputBackend::new();
    backend.set_mouse_velocity(Vec2::new(1000.0, 0.0));
    let got = evaluate(&look(), &backend).vector2();
    assert!(got.abs_diff_eq(Vec2::X, 1e-5), "read {got:?}");
}

/// Screen space counts down and a control counts up, as a stick's Y does: moving the mouse up
/// has to read the way pushing the stick up does, or one of the two is always inverted.
#[test]
fn mouse_up_reads_as_stick_up() {
    let mut backend = MockInputBackend::new();
    backend.set_mouse_velocity(Vec2::new(0.0, -1000.0));
    let mouse = evaluate(&look(), &backend).vector2();

    let mut pad = MockInputBackend::new();
    pad.add_gamepad(GamepadId(0));
    pad.set_axis(GamepadId(0), GamepadAxis::RightStickY, 1.0);
    let stick = evaluate(&look(), &pad).vector2();

    assert!(
        mouse.y > 0.0 && stick.y > 0.0,
        "mouse {mouse:?}, stick {stick:?}"
    );
}

/// One action, both devices: whichever is moving answers.
#[test]
fn the_stick_answers_the_same_look() {
    let mut backend = MockInputBackend::new();
    backend.add_gamepad(GamepadId(0));
    backend.set_axis(GamepadId(0), GamepadAxis::RightStickX, -0.5);
    let got = evaluate(&look(), &backend).vector2();
    assert!(got.abs_diff_eq(Vec2::new(-0.5, 0.0), 1e-5), "read {got:?}");
}

#[test]
fn a_still_mouse_reads_nothing() {
    let backend = MockInputBackend::new();
    assert_eq!(evaluate(&look(), &backend).vector2(), Vec2::ZERO);
}

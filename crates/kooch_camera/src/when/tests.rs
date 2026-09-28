//! What a camera's condition is worth, and when (#1352).

use super::*;

fn when(toggle: bool) -> CameraWhen {
    CameraWhen {
        boost: 10,
        toggle,
        ..Default::default()
    }
}

/// Held: worth its boost while asked, and nothing the moment it is not.
#[test]
fn a_held_ask_is_worth_its_boost() {
    let mut when = when(false);
    when.asked = true;
    when.step();
    assert_eq!(when.boost(), 10);

    when.asked = false;
    when.step();
    assert_eq!(when.boost(), 0);
}

/// Toggled: a press turns it on and it stays on with nothing held.
#[test]
fn a_toggle_latches_on_a_press() {
    let mut when = when(true);
    when.asked = true;
    when.step();
    assert_eq!(when.boost(), 10);

    // Still held: the same press, not a second one.
    when.step();
    assert_eq!(when.boost(), 10, "the hold toggled it twice");

    // Let go: a toggle does not end with the button.
    when.asked = false;
    when.step();
    assert_eq!(when.boost(), 10, "letting go turned it off");

    // And the next press ends it.
    when.asked = true;
    when.step();
    assert_eq!(when.boost(), 0);
}

/// Nothing asked is worth nothing, whatever the boost says.
#[test]
fn a_quiet_condition_is_worth_nothing() {
    let mut when = when(false);
    when.step();
    assert_eq!(when.boost(), 0);
}

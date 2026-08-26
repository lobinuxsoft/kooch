use kooch_core::resource::Resources;

use super::*;

#[test]
fn idle_delta_does_nothing_on_apply() {
    // No editor camera entity, no resources — must not panic.
    let mut resources = Resources::new();
    apply_viewport_input(ViewportInputDelta::default(), &mut resources, None);
}

#[test]
fn is_idle_detects_default() {
    assert!(ViewportInputDelta::default().is_idle());
}

#[test]
fn is_idle_detects_orbit_input() {
    let mut d = ViewportInputDelta::default();
    d.orbit_yaw = 0.01;
    assert!(!d.is_idle());
}

#[test]
fn is_idle_detects_fly_keys() {
    let mut d = ViewportInputDelta::default();
    d.fly_keys.forward = true;
    assert!(!d.is_idle());
}

#[test]
fn is_idle_detects_focus_press() {
    let mut d = ViewportInputDelta::default();
    d.focus_pressed = true;
    assert!(!d.is_idle());
}

#[test]
fn is_idle_detects_zoom() {
    let mut d = ViewportInputDelta::default();
    d.zoom_lines = -1.0;
    assert!(!d.is_idle());
}

use super::*;

const DT: f32 = 1.0 / 60.0;

fn lookahead() -> CameraLookahead {
    CameraLookahead {
        smoothing_duration: 0.0,
        ..Default::default()
    }
}

/// Runs a target along +X at `speed` for `steps`, then holds it still for `still`, returning the
/// last lead.
fn run(look: &CameraLookahead, speed: f32, steps: usize, still: usize) -> Vec3 {
    let mut lead = Lead::at(Vec3::ZERO);
    let mut target = Vec3::ZERO;
    let mut led = Vec3::ZERO;
    for _ in 0..steps {
        target.x += speed * DT;
        led = look.led(&mut lead, target, Vec3::Y, DT);
    }
    for _ in 0..still {
        led = look.led(&mut lead, target, Vec3::Y, DT);
    }
    led - target
}

#[test]
fn a_running_target_is_led() {
    // 6 m/s for 0.4 s ahead is 2.4 m.
    let lead = run(&lookahead(), 6.0, 60, 0);
    assert!((lead.x - 2.4).abs() < 1e-3, "{lead}");
}

#[test]
fn the_lead_is_capped() {
    let lead = run(&lookahead(), 60.0, 60, 0);
    assert!((lead.length() - 4.0).abs() < 1e-4, "{lead}");
}

#[test]
fn a_jump_is_not_led() {
    let look = lookahead();
    let mut lead = Lead::at(Vec3::ZERO);
    let led = look.led(&mut lead, Vec3::new(0.0, 0.5, 0.0), Vec3::Y, DT);
    assert_eq!(led, Vec3::new(0.0, 0.5, 0.0));
}

/// Stopping brings the framing back in exactly `smoothing_duration`, and not before: no snap.
#[test]
fn stopping_returns_on_time() {
    let look = CameraLookahead {
        smoothing_duration: 0.5,
        ..Default::default()
    };
    assert_eq!(run(&look, 6.0, 120, 30), Vec3::ZERO);
    assert!(run(&look, 6.0, 120, 29).x > 0.0, "back before its time");
    // One step after stopping it has barely moved: the lead eases out, it does not jump.
    let first = run(&look, 6.0, 120, 1).x;
    assert!(first > 2.0, "snapped back to {first}");
}

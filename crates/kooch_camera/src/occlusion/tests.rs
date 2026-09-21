use super::*;

const FRAME: f32 = 1.0 / 60.0;

/// Runs the arm from `from` towards a clear `to` for `seconds`, as frames would.
fn returned(from: f32, to: f32, return_time: f32, seconds: f32, dt: f32) -> f32 {
    let mut state = (from, None);
    let mut elapsed = 0.0;
    while elapsed + dt * 0.5 < seconds {
        state = arm_length(Some(state), to, return_time, dt);
        elapsed += dt;
    }
    state.0
}

/// 🔴 Pulling in is immediate: a camera that eased into a wall shows the inside of it for as long
/// as the ease takes.
#[test]
fn a_wall_pulls_in_at_once() {
    assert_eq!(arm_length(Some((5.0, None)), 1.2, 0.35, FRAME).0, 1.2);
}

/// 🔴 The return lasts what the field says, not three times it: an exponential covers 63 % of the
/// way in one time constant and looks like it never quite arrives.
#[test]
fn a_return_takes_its_seconds() {
    let halfway = returned(1.0, 5.0, 0.5, 0.25, FRAME);
    assert!(
        (halfway - 3.0).abs() < 0.05,
        "half the time, half the way: {halfway}"
    );
    let done = returned(1.0, 5.0, 0.5, 0.5, FRAME);
    assert_eq!(done, 5.0);
}

/// A zero return time is a camera that snaps back, which is what an author who typed zero asked for.
#[test]
fn a_zero_return_snaps_back() {
    assert_eq!(arm_length(Some((1.0, None)), 5.0, 0.0, FRAME).0, 5.0);
}

/// The first frame has nothing to return from.
#[test]
fn a_first_frame_takes_the_clear_length() {
    assert_eq!(arm_length(None, 3.0, 0.35, FRAME).0, 3.0);
}

/// Seconds are seconds at any frame rate.
#[test]
fn the_return_ignores_the_frame_rate() {
    let fast = returned(1.0, 5.0, 0.4, 0.2, 1.0 / 120.0);
    let slow = returned(1.0, 5.0, 0.4, 0.2, 1.0 / 30.0);
    assert!(
        (fast - slow).abs() < 0.05,
        "{fast} at 120 fps, {slow} at 30"
    );
}

/// A wall that comes back mid-return pulls in again at once, and the next return starts over.
#[test]
fn a_wall_mid_return_pulls_in() {
    let (length, returning) = arm_length(Some((2.0, Some((1.0, 0.1)))), 1.5, 0.5, FRAME);
    assert_eq!(length, 1.5);
    assert!(returning.is_none());
}

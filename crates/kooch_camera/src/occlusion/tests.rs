use super::*;

const FRAME: f32 = 1.0 / 60.0;

/// A collision easing back out on `seconds`.
fn over(seconds: f32) -> Deoccluder {
    Deoccluder {
        damping: seconds,
        ..Default::default()
    }
}

/// Runs the arm from `from` towards a clear `to` for `seconds`, as frames would.
fn returned(from: f32, to: f32, damping: f32, seconds: f32, dt: f32) -> f32 {
    let mut state = (from, None);
    let mut elapsed = 0.0;
    while elapsed + dt * 0.5 < seconds {
        state = arm_length(Some(state), to, &over(damping), dt);
        elapsed += dt;
    }
    state.0
}

/// 🔴 Pulling in is immediate: a camera that eased into a wall shows the inside of it for as long
/// as the ease takes.
#[test]
fn a_wall_pulls_in_at_once() {
    assert_eq!(
        arm_length(Some((5.0, None)), 1.2, &over(0.35), FRAME).0,
        1.2
    );
}

/// 🔴 #1339: a hundredth of the way left after its seconds, the same easing the rest of the rig
/// uses. Not a duration: the way clears a little at a time and the unobstructed arm moves with the
/// player, so the goal moves every frame — and a tween with a duration restarts on a moving goal.
#[test]
fn a_return_leaves_a_hundredth() {
    let left = (5.0 - returned(1.0, 5.0, 0.5, 0.5, FRAME)) / 4.0;
    assert!(
        (left - 0.01).abs() < 5e-3,
        "left {left} of the way, wanted a hundredth"
    );
}

/// A zero is a camera that snaps back, which is what an author who typed zero asked for.
#[test]
fn a_zero_return_snaps_back() {
    assert_eq!(arm_length(Some((1.0, None)), 5.0, &over(0.0), FRAME).0, 5.0);
}

/// The first frame has nothing to return from.
#[test]
fn a_first_frame_takes_the_clear_length() {
    assert_eq!(arm_length(None, 3.0, &over(0.35), FRAME).0, 3.0);
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

/// A wall that comes back mid-return pulls in again at once: going in has its own speed, and it is
/// zero by default.
#[test]
fn a_wall_mid_return_pulls_in() {
    let (length, _) = arm_length(Some((2.0, Some((1.0, 0.1)))), 1.5, &over(0.5), FRAME);
    assert_eq!(length, 1.5);
}

/// 🔴 Cinemachine's `DampingWhenOccluded`: a camera may ease INTO a wall too, if an author asks.
/// Zero — the default — is immediate, because easing in shows the inside of the wall while it runs.
#[test]
fn easing_in_is_its_own_speed() {
    let gentle = Deoccluder {
        damping: 0.35,
        damping_when_occluded: 0.5,
        ..Default::default()
    };
    let (length, _) = arm_length(Some((5.0, None)), 1.0, &gentle, FRAME);
    assert!(
        length > 4.0,
        "it went straight in despite being asked not to: {length}"
    );
}

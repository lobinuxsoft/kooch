use super::*;

/// 🔴 Pulling in is immediate: a camera that eased into a wall shows the inside of it for as long
/// as the ease takes.
#[test]
fn a_wall_pulls_in_at_once() {
    assert_eq!(arm_length(Some(5.0), 1.2, 0.35, 1.0 / 60.0), 1.2);
}

/// Going back out is eased, or walking past a doorway strobes the camera in and out.
#[test]
fn a_clear_way_eases_back_out() {
    let length = arm_length(Some(1.0), 5.0, 0.35, 1.0 / 60.0);
    assert!(
        length > 1.0 && length < 1.5,
        "one frame moved it to {length}"
    );
}

/// A zero return time is a camera that snaps back, which is what an author who typed zero asked for.
#[test]
fn a_zero_return_snaps_back() {
    assert_eq!(arm_length(Some(1.0), 5.0, 0.0, 1.0 / 60.0), 5.0);
}

/// The first frame has nothing to ease from.
#[test]
fn a_first_frame_takes_the_clear_length() {
    assert_eq!(arm_length(None, 3.0, 0.35, 1.0 / 60.0), 3.0);
}

/// The ease is a time constant: the same wall-clock time covers the same share of the way at any
/// frame rate.
#[test]
fn the_ease_ignores_the_frame_rate() {
    let mut fast = 1.0;
    for _ in 0..120 {
        fast = arm_length(Some(fast), 5.0, 0.35, 1.0 / 120.0);
    }
    let mut slow = 1.0;
    for _ in 0..30 {
        slow = arm_length(Some(slow), 5.0, 0.35, 1.0 / 30.0);
    }
    assert!(
        (fast - slow).abs() < 1e-3,
        "{fast} at 120 fps, {slow} at 30"
    );
}

//! Test code for `pages`, in its own file.

use super::{AGE_FRAMES_MAX, AGE_FRAMES_MIN, age_frames};

/// The horizon is a duration, so a faster renderer counts MORE
/// frames to reach the same second — the property the constant it
/// replaced could not have.
#[test]
fn a_faster_frame_holds_more_frames() {
    assert_eq!(age_frames(1.0, 1.0 / 60.0), 60);
    assert_eq!(age_frames(1.0, 1.0 / 150.0), 150);
    assert_eq!(age_frames(1.0, 1.0 / 240.0), 240);
}

#[test]
fn a_stall_cannot_evict_the_world() {
    // Half a second a frame would round to 2 without the floor,
    // and two frames of memory is a pool that thrashes on a hitch.
    assert_eq!(age_frames(1.0, 0.5), AGE_FRAMES_MIN);
}

#[test]
fn a_stopped_clock_cannot_be_immortal() {
    assert_eq!(age_frames(1.0, 1.0 / 100_000.0), AGE_FRAMES_MAX);
}

use super::*;

/// The chain ends at 1x1, and a square power of two is the easy case.
#[test]
fn a_square_power_of_two_chains_to_one() {
    assert_eq!(level_count(1024, 1024), 11);
    assert_eq!(level_count(1, 1), 1);
    assert_eq!(level_count(2, 2), 2);
}

/// 🔴 The count follows the LONGER side.
///
/// Levels halve both axes and the short one saturates at 1 while the
/// long one keeps going. Taking the shorter side — or the width, which
/// is the same mistake in a different disguise — stops the chain early
/// and leaves the last levels of a 512x64 texture unwritten: sampled
/// garbage at grazing angles, which reads as flickering seams rather
/// than as a missing level.
#[test]
fn the_longer_side_decides() {
    assert_eq!(level_count(512, 64), 10);
    assert_eq!(level_count(64, 512), 10);
}

/// Not every texture is a power of two, and the count rounds DOWN.
///
/// A 640x480 chain is 10 levels: 640 halves to 1 in nine steps
/// (320, 160, 80, 40, 20, 10, 5, 2, 1) and level zero makes ten. Rounding
/// up would ask wgpu for a level below 1x1, which it rejects outright —
/// the texture would fail to create rather than look wrong.
#[test]
fn a_non_power_of_two_rounds_down() {
    assert_eq!(level_count(640, 480), 10);
    assert_eq!(level_count(3, 3), 2);
    assert_eq!(level_count(1000, 1000), 10);
}

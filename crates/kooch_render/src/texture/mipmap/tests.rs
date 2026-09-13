use super::*;

/// The chain ends at 1x1, and a square power of two is the easy case.
#[test]
fn a_square_power_of_two_chains_to_one() {
    assert_eq!(level_count(1024, 1024), 11);
    assert_eq!(level_count(1, 1), 1);
    assert_eq!(level_count(2, 2), 2);
}

/// 🔴 The count follows the LONGER side.
#[test]
fn the_longer_side_decides() {
    assert_eq!(level_count(512, 64), 10);
    assert_eq!(level_count(64, 512), 10);
}

/// Not every texture is a power of two, and the count rounds DOWN.
#[test]
fn a_non_power_of_two_rounds_down() {
    assert_eq!(level_count(640, 480), 10);
    assert_eq!(level_count(3, 3), 2);
    assert_eq!(level_count(1000, 1000), 10);
}

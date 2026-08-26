use super::*;

/// The growth policy, without a device. What matters is that it
/// reaches the requirement and does not creep up one slot at a time.
fn grown(from: u32, required: u32) -> u32 {
    if required <= from {
        return from;
    }
    required
        .checked_next_power_of_two()
        .unwrap_or(required)
        .max(from.saturating_mul(2))
}

/// The case that panicked: 608 instances against the default 256.
#[test]
fn a_dense_scene_fits_after_growing() {
    assert!(grown(256, 608) >= 608);
}

#[test]
fn growth_is_geometric_not_incremental() {
    // A scene gaining one instance at a time must not reallocate on
    // every frame, so one growth has to leave real headroom.
    let after = grown(256, 257);
    assert!(
        after >= 512,
        "grew to {after}, which is one frame away from growing again",
    );
}

#[test]
fn a_capacity_that_already_fits_is_left_alone() {
    assert_eq!(grown(1024, 608), 1024);
    assert_eq!(grown(608, 608), 608);
}

/// `next_power_of_two` returns `None` past `2^31`; the fallback has
/// to be the requirement itself rather than a wrap to zero.
#[test]
fn an_enormous_requirement_does_not_wrap() {
    let huge = u32::MAX - 1;
    assert!(grown(256, huge) >= huge);
}

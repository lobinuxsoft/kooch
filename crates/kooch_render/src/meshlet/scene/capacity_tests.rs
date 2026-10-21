/// The real policy, without a device — reimplementing it here would pass while the engine
/// diverged, which is what this file used to do.
fn grown(from: u32, required: u32) -> u32 {
    match required <= from {
        true => from,
        false => kooch_core::gpu::grown(required, from),
    }
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

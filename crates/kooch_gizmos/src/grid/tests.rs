use super::{GridLevel, STEPS};

#[test]
fn close_up_shows_the_snap_step() {
    // Standing on it, the finest cell is the one the handles land on.
    let level = GridLevel::at(0.5, 0.5);
    assert_eq!(level.small_step, 0.5);
    assert_eq!(level.blend, 0.0);
}

#[test]
fn a_level_up_multiplies_by_the_steps() {
    // Ten steps away is one level out.
    let level = GridLevel::at(5.0, 0.5);
    assert!(
        (level.small_step - 5.0).abs() < 1e-4,
        "{}",
        level.small_step
    );
}

#[test]
fn the_blend_walks_from_zero_to_one() {
    // 🔴 The fraction is the crossfade. Without it the step jumps and
    // the grid pops.
    let start = GridLevel::at(0.5, 0.5).blend;
    let middle = GridLevel::at(1.6, 0.5).blend;
    let end = GridLevel::at(4.9, 0.5).blend;
    assert!(start < middle && middle < end, "{start} {middle} {end}");
    assert!(end < 1.0);
}

#[test]
fn the_blend_resets_at_each_level() {
    // Just past a boundary the fine lines are the coarse ones from
    // before, and the fade starts again.
    let before = GridLevel::at(4.99, 0.5);
    let after = GridLevel::at(5.01, 0.5);
    assert!(after.blend < before.blend);
    assert!((after.small_step - before.large_step()).abs() < 1e-3);
}

#[test]
fn the_coarse_level_is_ten_of_the_fine() {
    let level = GridLevel::at(2.0, 0.5);
    assert!((level.large_step() - level.small_step * STEPS).abs() < 1e-5);
}

#[test]
fn the_step_is_the_floor() {
    // Closer than one cell does not draw a finer grid than the handles
    // can land on.
    assert_eq!(GridLevel::at(0.001, 0.5).small_step, 0.5);
}

#[test]
fn a_zero_step_does_not_divide() {
    // No lattice to land on; refusing beats a NaN spreading to a shader.
    let level = GridLevel::at(10.0, 0.0);
    assert!(level.small_step.is_finite());
    assert_eq!(level.blend, 0.0);
}

#[test]
fn an_infinite_distance_is_refused() {
    let level = GridLevel::at(f32::INFINITY, 0.5);
    assert!(level.small_step.is_finite());
}

#[test]
fn a_smaller_unit_scales_with_it() {
    // A project in centimetres behaves like one in metres: level 0 is
    // its own step, not a hard-coded metre.
    let metres = GridLevel::at(5.0, 0.5);
    let centimetres = GridLevel::at(0.05, 0.005);
    assert!((metres.blend - centimetres.blend).abs() < 1e-4);
}

#[test]
fn a_fixed_level_ignores_distance() {
    // The guide draws the step a drag moves by. Pulling the camera back
    // must not coarsen it into a distance no handle can land on.
    let near = GridLevel::fixed(0.5);
    let far = GridLevel::fixed(0.5);
    assert_eq!(near.small_step, 0.5);
    assert_eq!(near.blend, 0.0);
    assert_eq!(near, far);
    // And it is NOT what the scaling one would give from up high.
    assert_ne!(GridLevel::at(500.0, 0.5).small_step, near.small_step);
}

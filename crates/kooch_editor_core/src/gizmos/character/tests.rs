use super::*;
use crate::gizmos::harness::draw;
use glam::Mat4;

/// The reach is the number an author edits, so it has to be the number
/// the drawing changes with.
#[test]
fn the_probe_sets_how_far_it_draws() {
    let shallow = CharacterController {
        probe: 1.0,
        ..Default::default()
    };
    let deep = CharacterController {
        probe: 6.0,
        ..Default::default()
    };
    let lowest = |controller| {
        draw(&CharacterVisualizer, &controller, Mat4::IDENTITY)
            .iter()
            .flat_map(|(a, b)| [a.y, b.y])
            .fold(f32::MAX, f32::min)
    };
    assert!(lowest(shallow) > lowest(deep) + 4.0);
}

/// A probe that stops before the rest height can never find the floor.
/// The extra marks are the whole warning.
#[test]
fn a_probe_that_cannot_reach_is_marked() {
    let sane = CharacterController::default();
    let broken = CharacterController {
        probe: 0.2,
        ride_height: 1.1,
        ..Default::default()
    };
    let count = |controller| draw(&CharacterVisualizer, &controller, Mat4::IDENTITY).len();
    assert!(
        count(broken) > count(sane),
        "the impossible one should draw more, not less",
    );
}

/// Everything is written against the local up, so a rig on its side
/// draws on its side.
#[test]
fn it_draws_along_the_local_up() {
    let controller = CharacterController::default();
    let segments = draw(&CharacterVisualizer, &controller, Mat4::IDENTITY);
    let lowest = segments
        .iter()
        .flat_map(|(a, b)| [a.y, b.y])
        .fold(f32::MAX, f32::min);
    assert!(lowest < -1.0, "it should reach below the origin: {lowest}");
}

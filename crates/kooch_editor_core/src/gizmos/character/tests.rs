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

/// Everything `draw_at` drew, given a collider reach.
fn with_reach(controller: &CharacterController, reach: Option<f32>) -> Vec<(Vec3, Vec3)> {
    use kooch_gizmos::{GizmoBatch, Gizmos, MeshBatch};
    let mut lines = GizmoBatch::default();
    let mut meshes = MeshBatch::default();
    {
        let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
        draw_at(
            controller,
            &GlobalTransform {
                matrix: Mat4::IDENTITY,
            },
            Vec3::Y,
            reach,
            &mut gizmos,
        );
    }
    lines.lines.iter().map(|s| (s.start, s.end)).collect()
}

/// The mistake the whole gizmo exists to catch: a capsule that reaches
/// further down than the height it is asked to ride at rests on the
/// floor, and nothing in either Inspector says so.
#[test]
fn a_collider_that_cannot_float_is_marked() {
    let controller = CharacterController::default();
    let clears = with_reach(&controller, Some(0.9)).len();
    let sinks = with_reach(&controller, Some(1.22)).len();
    assert!(
        sinks > clears,
        "a reach past the ride height should draw the warning: {sinks} vs {clears}",
    );
}

/// And without a collider to compare against it says nothing, rather
/// than drawing a reach of zero — which would read as "this clears".
#[test]
fn an_unknown_reach_draws_nothing() {
    let controller = CharacterController::default();
    assert!(with_reach(&controller, None).len() < with_reach(&controller, Some(0.9)).len());
}

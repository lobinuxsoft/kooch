use super::*;
use crate::gizmos::harness::draw;
use glam::{Mat4, Quat};

/// The steering arrow is the one an author is looking for, so it has to
/// follow the component rather than the body.
#[test]
fn it_points_where_it_is_steered() {
    let facing = Facing { direction: Vec3::X };
    let far = draw(&FacingVisualizer, &facing, Mat4::IDENTITY)
        .iter()
        .map(|(_, b)| b.x)
        .fold(f32::MIN, f32::max);
    assert!(far > 1.0, "should reach along +X: {far}");
}

/// Two arrows while they disagree, one while they agree — which is the
/// whole reason both are drawn.
#[test]
fn a_disagreement_draws_twice() {
    let facing = Facing {
        direction: Vec3::NEG_Z,
    };
    let agreed = draw(&FacingVisualizer, &facing, Mat4::IDENTITY).len();
    let turned = Mat4::from_quat(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
    let apart = draw(&FacingVisualizer, &facing, turned);
    assert_eq!(agreed, apart.len(), "both draw two arrows");
    let along_x = apart.iter().any(|(_, b)| b.x.abs() > 1.0);
    let along_z = apart.iter().any(|(_, b)| b.z.abs() > 1.0);
    assert!(along_x && along_z, "they should point different ways");
}

/// No steering is not steering at zero: the controller keeps the heading
/// it has, and an arrow would invent an intent that was never written.
#[test]
fn no_steering_draws_no_arrow() {
    let quiet = Facing {
        direction: Vec3::ZERO,
    };
    let steered = Facing { direction: Vec3::X };
    assert!(
        draw(&FacingVisualizer, &quiet, Mat4::IDENTITY).len()
            < draw(&FacingVisualizer, &steered, Mat4::IDENTITY).len()
    );
}

use super::*;
use crate::gizmos::harness::draw;
use glam::Mat4;

/// Nothing found means nothing drawn. A contact at the origin would read
/// as ground at your feet.
#[test]
fn no_ground_draws_nothing() {
    let segments = draw(&GroundedVisualizer, &Grounded::default(), Mat4::IDENTITY);
    assert!(segments.is_empty(), "{} segments", segments.len());
}

/// The gap is the number the spring is holding, so the drawing has to
/// sit at it.
#[test]
fn the_contact_sits_at_the_measured_gap() {
    let found = Grounded {
        standing: true,
        normal: Vec3::Y,
        distance: 2.5,
    };
    let lowest = draw(&GroundedVisualizer, &found, Mat4::IDENTITY)
        .iter()
        .flat_map(|(a, b)| [a.y, b.y])
        .fold(f32::MAX, f32::min);
    assert!(
        (lowest + 2.5).abs() < 0.3,
        "drew down to {lowest}, wanted -2.5"
    );
}

/// A wall is found and refused, and the two have to look different or
/// the drawing answers the wrong question.
#[test]
fn refused_ground_is_still_drawn() {
    let wall = Grounded {
        standing: false,
        normal: Vec3::X,
        distance: 1.0,
    };
    assert!(!draw(&GroundedVisualizer, &wall, Mat4::IDENTITY).is_empty());
}

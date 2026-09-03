use super::*;
use crate::gizmos::harness::draw;
use glam::Mat4;

/// No wall draws nothing. A contact at the origin would read as a wall
/// the character is standing inside.
#[test]
fn nothing_found_draws_nothing() {
    assert!(draw(&TouchingVisualizer, &Touching::default(), Mat4::IDENTITY).is_empty());
}

/// The contact is where the probe stopped, not where the body is.
#[test]
fn it_draws_at_the_contact() {
    let found = Touching {
        wall: true,
        normal: Vec3::NEG_X,
        distance: 0.7,
    };
    let far = draw(&TouchingVisualizer, &found, Mat4::IDENTITY)
        .iter()
        .flat_map(|(a, b)| [a.x, b.x])
        .fold(f32::MIN, f32::max);
    assert!(far > 0.6, "should reach out to the wall at +X: {far}");
}

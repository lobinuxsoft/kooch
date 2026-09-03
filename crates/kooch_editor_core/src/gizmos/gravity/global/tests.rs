use super::*;
use crate::gizmos::harness::{draw, shaft};
use glam::{Mat4, Quat};

#[test]
fn a_uniform_field_draws_along_its_acceleration() {
    let field = GlobalGravity::default();
    let segments = draw(&GlobalGravityVisualizer, &field, Mat4::IDENTITY);
    assert!(shaft(&segments).abs_diff_eq(Vec3::NEG_Y, 1e-3));
}

/// `acceleration` is a world vector. Deriving the arrows from the
/// entity's basis would be the natural thing to write and would make
/// the gizmo disagree with the solver the moment anyone rotated it.
#[test]
fn a_uniform_field_does_not_turn_with_its_entity() {
    let field = GlobalGravity::default();
    let turned = Mat4::from_quat(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
    assert!(shaft(&draw(&GlobalGravityVisualizer, &field, turned)).abs_diff_eq(Vec3::NEG_Y, 1e-3));
}

#[test]
fn a_degenerate_field_draws_nothing() {
    let field = GlobalGravity {
        acceleration: Vec3::ZERO,
    };
    assert!(draw(&GlobalGravityVisualizer, &field, Mat4::IDENTITY).is_empty());
}

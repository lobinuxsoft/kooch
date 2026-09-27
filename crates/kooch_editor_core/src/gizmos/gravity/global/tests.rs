use super::*;
use crate::gizmos::harness::{arrows, draw};
use glam::{Mat4, Quat};

#[test]
fn a_uniform_field_draws_along_its_acceleration() {
    let field = GlobalGravity::default();
    let drawn = arrows(&GlobalGravityVisualizer, &field, Mat4::IDENTITY);
    assert!(!drawn.is_empty(), "a uniform field drew no arrow");
    for arrow in drawn {
        assert!(arrow.abs_diff_eq(Vec3::NEG_Y, 1e-3), "{arrow}");
    }
}

/// `acceleration` is a world vector. Deriving the arrows from the
/// entity's basis would be the natural thing to write and would make
/// the gizmo disagree with the solver the moment anyone rotated it.
#[test]
fn a_uniform_field_does_not_turn_with_its_entity() {
    let field = GlobalGravity::default();
    let turned = Mat4::from_quat(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
    let turned = arrows(&GlobalGravityVisualizer, &field, turned);
    assert!(!turned.is_empty(), "a uniform field drew no arrow");
    for arrow in turned {
        assert!(arrow.abs_diff_eq(Vec3::NEG_Y, 1e-3), "{arrow}");
    }
}

#[test]
fn a_degenerate_field_draws_nothing() {
    let field = GlobalGravity {
        acceleration: Vec3::ZERO,
    };
    assert!(draw(&GlobalGravityVisualizer, &field, Mat4::IDENTITY).is_empty());
    assert!(arrows(&GlobalGravityVisualizer, &field, Mat4::IDENTITY).is_empty());
}

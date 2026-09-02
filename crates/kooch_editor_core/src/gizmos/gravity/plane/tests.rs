use super::*;
use crate::gizmos::gravity::harness::{draw, reach, shafts};
use glam::{Mat4, Quat, Vec3};

/// A floor pulls down. Pointing along the normal would read as a
/// repulsor, which is the one thing this component is not.
#[test]
fn a_plane_points_away_from_its_normal() {
    let field = PlaneGravity::default();
    let shafts = shafts(&draw(&PlaneGravityVisualizer, &field, Mat4::IDENTITY));
    assert_eq!(shafts.len(), 4, "expected one arrow per corner");
    for shaft in shafts {
        assert!((shaft - Vec3::NEG_Y).length() < 1e-3, "{shaft}");
    }
}

/// `normal` is local, so this is the drawing that shows a tipped floor
/// actually tipped — the exact thing that is invisible otherwise.
#[test]
fn a_plane_rotates_with_its_entity() {
    let matrix = Mat4::from_quat(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
    let shafts = shafts(&draw(
        &PlaneGravityVisualizer,
        &PlaneGravity::default(),
        matrix,
    ));
    for shaft in shafts {
        assert!((shaft - Vec3::X).length() < 1e-3, "{shaft}");
    }
}

/// The heights are the numbers an author edits, so they have to be the
/// numbers the drawing changes with.
#[test]
fn a_plane_draws_out_to_its_falloff() {
    let field = PlaneGravity {
        range: 100.0,
        falloff: 20.0,
        ..Default::default()
    };
    let reach = reach(&draw(&PlaneGravityVisualizer, &field, Mat4::IDENTITY));
    assert!(
        (reach - 120.0).abs() < PATCH * 1.5,
        "reached {reach}, wanted 120"
    );
}

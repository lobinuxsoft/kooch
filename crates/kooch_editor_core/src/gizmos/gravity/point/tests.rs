use super::*;
use crate::gizmos::harness::{draw, reach};
use glam::Mat4;

/// Where the field reaches zero is the outer sphere — past the radius by its fade — so the gizmo
/// has to reach it.
#[test]
fn a_point_source_draws_out_to_its_fade() {
    let field = PointGravity {
        radius: 100.0,
        falloff: 10.0,
        ..Default::default()
    };
    let reach = reach(&draw(&PointGravityVisualizer, &field, Mat4::IDENTITY));
    assert!((reach - 110.0).abs() < 1.0, "reached {reach}, wanted 110");
}

/// A zero radius is unlimited, and there is no sphere for infinity — so the drawing is the arrows
/// alone.
#[test]
fn an_unlimited_point_source_draws_only_arrows() {
    let field = PointGravity {
        radius: 0.0,
        ..Default::default()
    };
    let reach = reach(&draw(&PointGravityVisualizer, &field, Mat4::IDENTITY));
    assert!(
        (reach - ARROW).abs() < 1e-3,
        "reached {reach}, wanted {ARROW}"
    );
}

/// A planet pulls inward. If the arrows pointed out it would read as a
/// repulsor, which is the one thing this component is not.
#[test]
fn a_point_source_points_inward() {
    let field = PointGravity {
        radius: 0.0,
        ..Default::default()
    };
    let segments = draw(&PointGravityVisualizer, &field, Mat4::IDENTITY);
    let shafts: Vec<_> = segments
        .iter()
        .filter(|(a, b)| ((*b - *a).length() - ARROW).abs() < 1e-3)
        .collect();
    assert_eq!(shafts.len(), 6, "expected one arrow per axis");
    for (a, b) in shafts {
        assert!(
            b.length() < a.length(),
            "an arrow from {a} to {b} points away from the centre",
        );
    }
}

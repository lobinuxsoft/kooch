use super::*;
use crate::gizmos::harness::{draw, reach};
use glam::Mat4;

/// The cutoff is the outer sphere, so the gizmo has to reach it.
#[test]
fn a_point_source_draws_out_to_its_range() {
    let field = PointGravity {
        radius: 10.0,
        range: 100.0,
        ..Default::default()
    };
    let reach = reach(&draw(&PointGravityVisualizer, &field, Mat4::IDENTITY));
    assert!((reach - 100.0).abs() < 1.0, "reached {reach}, wanted 100");
}

/// Zero range means unlimited, and there is no sphere for infinity —
/// so the drawing must stop at the radius rather than at zero.
#[test]
fn an_unlimited_point_source_draws_only_its_radius() {
    let field = PointGravity {
        radius: 10.0,
        range: 0.0,
        ..Default::default()
    };
    let reach = reach(&draw(&PointGravityVisualizer, &field, Mat4::IDENTITY));
    assert!((reach - 10.0).abs() < 1.0, "reached {reach}, wanted 10");
}

/// A planet pulls inward. If the arrows pointed out it would read as a
/// repulsor, which is the one thing this component is not.
#[test]
fn a_point_source_points_inward() {
    let field = PointGravity {
        radius: 10.0,
        range: 0.0,
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

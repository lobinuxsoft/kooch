use super::*;
use crate::gizmos::harness::{draw, drawn_arrows, reach};
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
    // Nothing but arrows: no cutoff shell among the lines, and the arrows reach `ARROW` out.
    assert!(
        draw(&PointGravityVisualizer, &field, Mat4::IDENTITY).is_empty(),
        "an unlimited source drew a boundary it does not have",
    );
    let far = drawn_arrows(&PointGravityVisualizer, &field, Mat4::IDENTITY)
        .iter()
        .map(|(base, _)| base.length())
        .fold(0.0, f32::max);
    assert!((far - ARROW).abs() < 1e-3, "reached {far}, wanted {ARROW}");
}

/// A planet pulls inward. If the arrows pointed out it would read as a
/// repulsor, which is the one thing this component is not.
#[test]
fn a_point_source_points_inward() {
    let field = PointGravity {
        radius: 0.0,
        ..Default::default()
    };
    let shafts = drawn_arrows(&PointGravityVisualizer, &field, Mat4::IDENTITY);
    assert_eq!(shafts.len(), 6, "expected one arrow per axis");
    for (base, tip) in shafts {
        assert!(
            tip.length() < base.length(),
            "an arrow from {base} to {tip} points away from the centre",
        );
    }
}

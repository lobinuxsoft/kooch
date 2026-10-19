use super::*;
use crate::spline::{TANGENT_AUTO, arc as length_of, eval};

/// 🔴 The number the issue asks for, and the one that decides whether `DEFAULT_SEGMENTS` is
/// honest. Catmull-Rom through points on a circle bulges **outwards** between them, so the worst
/// error is at the middle of a segment. Measured against a unit radius.
#[test]
fn a_circle_is_round_enough() {
    let points = circle(1.0, DEFAULT_SEGMENTS);
    let worst = (0..DEFAULT_SEGMENTS)
        .map(|segment| {
            // Halfway along one segment, where the bulge is greatest.
            let t = (segment as f32 + 0.5) / DEFAULT_SEGMENTS as f32;
            (eval::at(&points, true, t).length() - 1.0).abs()
        })
        .fold(0.0_f32, f32::max);
    assert!(worst < 0.002, "a 12-segment circle is off by {worst}");
}

/// Every knot the same distance out, or it is not a circle however smooth the curve between them.
#[test]
fn a_circle_is_centred() {
    let points = circle(3.0, 8);
    assert_eq!(points.len(), 8);
    for knot in &points {
        assert!(
            (knot.position.length() - 3.0).abs() < 1.0e-4,
            "{:?} is not 3 m out",
            knot.position
        );
        assert!(knot.position.y.abs() < 1.0e-6, "a circle left the XZ plane");
        assert_eq!(knot.mode, TANGENT_AUTO);
    }
}

/// 🔴 A ring's last knot must not repeat its first. The closing segment is already a segment, so a
/// duplicate makes a zero-length one — which `eval` divides by and `arc` measures as nothing.
#[test]
fn a_ring_does_not_repeat() {
    let points = circle(1.0, 6);
    let first = points[0].position;
    let last = points[points.len() - 1].position;
    assert!(first.distance(last) > 0.5, "the ring closed on itself");
}

/// An arc has ends, so it needs a knot on each: `segments + 1`.
#[test]
fn an_arc_keeps_both_ends() {
    let quarter = arc(2.0, std::f32::consts::FRAC_PI_2, 4);
    assert_eq!(quarter.len(), 5);
    assert!((quarter[0].position - Vec3::new(2.0, 0.0, 0.0)).length() < 1.0e-4);
    assert!((quarter[4].position - Vec3::new(0.0, 0.0, 2.0)).length() < 1.0e-4);
}

/// A sweep of a full turn is the circle, and must not then carry a knot on top of its first.
#[test]
fn a_whole_sweep_is_a_ring() {
    assert_eq!(arc(1.0, TAU, 8).len(), 8);
    assert_eq!(arc(1.0, -TAU, 8).len(), 8);
}

/// The staircase case: it climbs, it turns, and the radius goes where it was asked to.
#[test]
fn a_spiral_climbs_and_turns() {
    let points = spiral(1.0, 2.0, 4.0, 2.0, 8);
    let first = points[0].position;
    let last = points[points.len() - 1].position;
    assert!(first.y.abs() < 1.0e-4, "{first:?}");
    assert!((last.y - 4.0).abs() < 1.0e-3, "{last:?}");
    assert!((Vec3::new(first.x, 0.0, first.z).length() - 1.0).abs() < 1.0e-4);
    assert!((Vec3::new(last.x, 0.0, last.z).length() - 2.0).abs() < 1.0e-3);
    // Two turns of a 1..2 m helix climbing 4 m is a good deal longer than the climb alone.
    assert!(length_of::length(&points, false) > 10.0);
}

/// 🔴 Every state an author passes through while dragging a number, and none of them may panic or
/// put a NaN in the scene — a curve of NaN draws nothing with no error to say why.
#[test]
fn degenerate_parameters_are_usable() {
    let cases = vec![
        circle(0.0, 0),
        circle(f32::NAN, 8),
        arc(1.0, 0.0, 0),
        arc(f32::NAN, f32::NAN, 1),
        spiral(0.0, 0.0, 0.0, 0.0, 0),
        spiral(f32::NAN, 1.0, f32::NAN, f32::NAN, 4),
        spiral(1.0, 1.0, 1.0, -3.0, 4),
    ];
    for points in cases {
        assert!(points.len() >= MIN_SEGMENTS, "{} knots", points.len());
        assert!(
            points.iter().all(|knot| knot.position.is_finite()),
            "a NaN reached the knots",
        );
    }
}

/// The list the author has to edit is bounded whatever they type.
#[test]
fn a_huge_count_is_clamped() {
    assert!(circle(1.0, 100_000).len() <= MAX_KNOTS);
    assert!(spiral(1.0, 1.0, 1.0, 1000.0, 512).len() <= MAX_KNOTS);
}

/// The measurement behind the doc comment's percentages. Printed, not asserted.
#[test]
#[ignore = "a measurement, not a check"]
fn circle_error_by_segments() {
    for segments in [6, 8, 12, 16, 24, 32] {
        let points = circle(1.0, segments);
        let worst = (0..segments * 4)
            .map(|step| {
                let t = step as f32 / (segments * 4) as f32;
                (eval::at(&points, true, t).length() - 1.0).abs()
            })
            .fold(0.0_f32, f32::max);
        println!("{segments} segments: {:.4} %", worst * 100.0);
    }
}

use super::*;
use crate::spline::{DEFAULT_RADIUS, TANGENT_BROKEN};

/// One long segment and one short one, in a straight line. Straight on purpose: the chord equals
/// the arc, so a spacing test measures the distribution and nothing else. By `t` the short 2 m
/// stretch takes half the points; by length it takes a sixth.
fn uneven() -> Vec<Knot> {
    [
        Vec3::ZERO,
        Vec3::new(10.0, 0.0, 0.0),
        Vec3::new(12.0, 0.0, 0.0),
    ]
    .map(Knot::at)
    .to_vec()
}

/// An L with a hairpin at the corner, for the cases that need real curvature.
fn bent() -> Vec<Knot> {
    [
        Vec3::ZERO,
        Vec3::new(10.0, 0.0, 0.0),
        Vec3::new(10.0, 1.0, 0.0),
    ]
    .map(Knot::at)
    .to_vec()
}

fn line() -> Vec<Knot> {
    [Vec3::ZERO, Vec3::new(4.0, 0.0, 0.0)]
        .map(Knot::at)
        .to_vec()
}

/// Gaps between consecutive knots.
fn gaps(knots: &[Knot]) -> Vec<f32> {
    knots
        .windows(2)
        .map(|pair| pair[0].position.distance(pair[1].position))
        .collect()
}

/// 🔴 The whole point of the operation. Spacing by `t` on this curve puts half the points in the
/// last sixth of it, and an extrude or a scatter built on that bunches up wherever the author
/// placed knots close together.
#[test]
fn a_resample_spaces_by_length() {
    let points = uneven();
    let spacing = gaps(&resample(&points, false, 7));
    let (smallest, largest) = (
        spacing.iter().copied().fold(f32::MAX, f32::min),
        spacing.iter().copied().fold(0.0, f32::max),
    );
    assert!(
        largest - smallest < 0.01,
        "spacing ran {smallest}..{largest}, so it is not by length",
    );

    // Teeth: the same count spaced by `t` is what this must not look like.
    let by_parameter: Vec<f32> = gaps(
        &(0..7)
            .map(|index| at_parameter(&points, false, index as f32 / 6.0))
            .collect::<Vec<_>>(),
    );
    let widest = by_parameter.iter().copied().fold(0.0f32, f32::max);
    let tightest = by_parameter.iter().copied().fold(f32::MAX, f32::min);
    assert!(
        widest / tightest > 2.0,
        "the curve is too even to prove anything: {by_parameter:?}",
    );
}

/// 🔴 The endpoints are the curve's own, or a resampled path stops short of where it went.
#[test]
fn a_resample_keeps_its_ends() {
    let points = bent();
    let knots = resample(&points, false, 6);
    assert!(knots[0].position.distance(points[0].position) < 1e-3);
    assert!(knots[5].position.distance(points[2].position) < 1e-3);
}

/// A ring has as many gaps as knots, so repeating the first at the end would file a zero-length
/// segment and double a point every scatter lands on.
#[test]
fn a_closed_resample_does_not_repeat() {
    let ring = [
        Vec3::ZERO,
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 1.0),
        Vec3::new(0.0, 0.0, 1.0),
    ]
    .map(Knot::at);
    let knots = resample(&ring, true, 8);
    assert_eq!(knots.len(), 8);
    assert!(knots[7].position.distance(knots[0].position) > 0.1);
}

/// Roll and radius are authored per point, so an operation that rebuilds the list has to carry
/// them across — a resampled curve that forgets its radius sweeps a different tube.
#[test]
fn a_resample_carries_radius() {
    let mut points = line();
    points[0].radius = 1.0;
    points[1].radius = 3.0;
    points[0].roll = 0.0;
    points[1].roll = 90.0;
    let middle = resample(&points, false, 3)[1];
    assert!((middle.radius - 2.0).abs() < 1e-3, "{}", middle.radius);
    assert!((middle.roll - 45.0).abs() < 1e-2, "{}", middle.roll);
}

#[test]
fn a_subdivide_counts_per_segment() {
    // Two segments, two cuts each: 3 authored + 4 inserted.
    assert_eq!(subdivide(&bent(), false, 2).len(), 7);
    // Closed adds the returning segment, so one more set of cuts and no repeated end.
    assert_eq!(subdivide(&bent(), true, 2).len(), 9);
}

/// Inserted points sit ON the curve, or subdividing is a way to change a shape by asking for
/// detail in it.
#[test]
fn a_subdivide_stays_on_curve() {
    let points = bent();
    let cut = subdivide(&points, false, 1)[1];
    let nearest = crate::spline::nearest::nearest_point(&points, false, cut.position);
    assert!(cut.position.distance(nearest) < 1e-3);
}

#[test]
fn a_reverse_flips_the_order() {
    let points = bent();
    let back = reverse(&points);
    assert_eq!(back[0].position, points[2].position);
    assert_eq!(back[2].position, points[0].position);
}

/// 🔴 The roll is about the tangent, and the tangent now points the other way. Keeping the number
/// banks the curve to the wrong side — invisible in a position test, obvious on a road.
#[test]
fn a_reverse_negates_the_roll() {
    let mut points = line();
    points[0].roll = 30.0;
    let back = reverse(&points);
    assert_eq!(back[1].roll, -30.0);
}

/// Both handles are authored pointing away from their knot, so reversing swaps which one the
/// curve arrives on without turning either around.
#[test]
fn a_reverse_swaps_the_handles() {
    let mut points = line();
    points[0].mode = TANGENT_BROKEN;
    points[0].leaving = Vec3::X;
    points[0].arriving = Vec3::Y;
    let back = reverse(&points)[1];
    assert_eq!(back.leaving, Vec3::Y);
    assert_eq!(back.arriving, Vec3::X);
}

/// ⚠️ The tolerance is `arc`'s, not this operation's. The table walks 16 steps per segment and a
/// 2-knot curve eases in and out, so inverting a distance lands within about 2 mm over 4 m.
/// Tightening this test means raising `STEPS_PER_SEGMENT`, which every caller pays for.
#[test]
fn a_trim_cuts_to_length() {
    let knots = trim(&line(), false, 1.0, 3.0);
    assert!(knots[0].position.distance(Vec3::new(1.0, 0.0, 0.0)) < 5e-3);
    let last = knots[knots.len() - 1].position;
    assert!(last.distance(Vec3::new(3.0, 0.0, 0.0)) < 5e-3);
}

/// A range given backwards is the same range. Refusing it would make a drag that crossed its own
/// start empty the curve.
#[test]
fn a_backwards_trim_is_the_same() {
    let gaps = |a, b| gaps(&trim(&line(), false, a, b));
    assert_eq!(gaps(1.0, 3.0), gaps(3.0, 1.0));
}

/// 🔴 A ring cut anywhere is a line. The caller sets `closed` itself, and this is the note that
/// says it must set it to false — the test exists so the contract is checked, not just written.
#[test]
fn a_trimmed_ring_has_two_ends() {
    let ring = [
        Vec3::ZERO,
        Vec3::new(2.0, 0.0, 0.0),
        Vec3::new(2.0, 0.0, 2.0),
        Vec3::new(0.0, 0.0, 2.0),
    ]
    .map(Knot::at);
    let knots = trim(&ring, true, 1.0, 5.0);
    assert!(knots.len() >= 2);
    assert!(knots[0].position.distance(knots[knots.len() - 1].position) > 0.5);
}

#[test]
fn an_empty_trim_gives_nothing() {
    assert!(trim(&line(), false, 2.0, 2.0).is_empty());
}

/// The corner is replaced by the two points the curve bends between, so the sharp vertex is gone.
#[test]
fn a_fillet_rounds_the_corner() {
    let corner = [
        Vec3::ZERO,
        Vec3::new(4.0, 0.0, 0.0),
        Vec3::new(4.0, 0.0, 4.0),
    ]
    .map(Knot::at);
    let knots = fillet(&corner, false, 1.0);
    assert_eq!(knots.len(), 4);
    assert!(
        knots
            .iter()
            .all(|knot| knot.position.distance(corner[1].position) > 0.5),
        "the corner survived the fillet",
    );
}

/// 🔴 Two corners filleted off one short segment would otherwise cross and fold the curve back on
/// itself. Clamping each pull to half the segment is what keeps the order of the knots.
#[test]
fn a_fillet_cannot_cross_itself() {
    let tight = [
        Vec3::ZERO,
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.1, 0.0, 0.0),
        Vec3::new(1.1, 0.0, 1.0),
    ]
    .map(Knot::at);
    let knots = fillet(&tight, false, 10.0);
    let along: Vec<f32> = knots.iter().map(|knot| knot.position.x).collect();
    assert!(
        along.windows(2).all(|pair| pair[1] >= pair[0] - 1e-4),
        "the knots went backwards: {along:?}",
    );
}

/// An end has one neighbour, so there is no corner there to round — and moving it would shorten
/// the curve the author drew.
#[test]
fn a_fillet_leaves_the_ends() {
    let corner = [
        Vec3::ZERO,
        Vec3::new(4.0, 0.0, 0.0),
        Vec3::new(4.0, 0.0, 4.0),
    ]
    .map(Knot::at);
    let knots = fillet(&corner, false, 1.0);
    assert_eq!(knots[0].position, Vec3::ZERO);
    assert_eq!(knots[3].position, Vec3::new(4.0, 0.0, 4.0));
}

/// 🔴 Nothing here may panic or answer with NaN on a curve that is not one. The editor runs these
/// on whatever is selected, and a spline with one knot is what a curve looks like while it is
/// being drawn.
#[test]
fn a_degenerate_curve_survives() {
    let stack = vec![Knot::at(Vec3::ZERO); 3];
    for knots in [
        resample(&[], false, 8),
        resample(&line()[..1], false, 8),
        resample(&stack, false, 8),
        subdivide(&stack, false, 3),
        subdivide(&[], true, 3),
        trim(&stack, false, 0.0, 1.0),
        fillet(&stack, false, 1.0),
        reverse(&stack),
    ] {
        assert!(
            knots
                .iter()
                .all(|knot| knot.position.is_finite() && knot.radius.is_finite()),
            "an operation answered with NaN",
        );
    }
}

/// A radius left at NaN by a bad import must not travel into the curve an operation hands back.
#[test]
fn a_nan_radius_cannot_fold_a_fillet() {
    let mut points = bent();
    points[1].position.y = f32::NAN;
    let knots = fillet(&points, false, 1.0);
    assert_eq!(
        knots.len(),
        3,
        "a corner with no direction was still pulled"
    );
}

#[test]
fn an_unasked_operation_changes_nothing() {
    let points = bent();
    assert_eq!(subdivide(&points, false, 0), points);
    assert_eq!(fillet(&points, false, 0.0), points);
    assert_eq!(resample(&points, false, 1), points);
    assert_eq!(points[0].radius, DEFAULT_RADIUS);
}

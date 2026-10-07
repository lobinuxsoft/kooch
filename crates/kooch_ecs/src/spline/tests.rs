use super::*;
use glam::Vec3;

/// A straight line of knots: `t` and distance agree, which is the only shape where they do.
fn straight() -> Vec<Knot> {
    (0..4)
        .map(|i| Knot::at(Vec3::new(i as f32, 0.0, 0.0)))
        .collect()
}

/// 🔴 The invariant every consumer leans on: equal steps of `t` are NOT equal steps along the path,
/// so #1428 spacing by `t` would bunch collectibles wherever the curve bends.
#[test]
fn a_bend_spends_t_unevenly() {
    let bent = vec![
        Knot::at(Vec3::ZERO),
        Knot::at(Vec3::new(10.0, 0.0, 0.0)),
        Knot::at(Vec3::new(10.1, 0.0, 0.0)),
    ];
    let total = arc::length(&bent, false);

    // Half the parameter is the first of two segments, which is almost all of the length.
    let half_t = eval::at(&bent, false, 0.5).distance(Vec3::ZERO);
    let half_way = arc::at_distance(&bent, false, total * 0.5).distance(Vec3::ZERO);

    assert!(
        half_t > half_way * 1.5,
        "t={half_t} vs distance={half_way} — they should disagree sharply"
    );
}

/// Distance is the thing #1428 spaces by, so it has to be measured and not assumed.
#[test]
fn a_straight_line_measures_true() {
    let points = straight();
    assert!((arc::length(&points, false) - 3.0).abs() < 1e-3);

    let middle = arc::at_distance(&points, false, 1.5);
    assert!(middle.distance(Vec3::new(1.5, 0.0, 0.0)) < 1e-3, "{middle}");
}

/// 🔴 What #1429 depends on: the up never flips. Frenet's normal would invert through the inflection
/// this S-curve has, and that is a character thrown off the road in one frame.
#[test]
fn an_inflection_keeps_the_up() {
    let curved = vec![
        Knot::at(Vec3::new(0.0, 0.0, 0.0)),
        Knot::at(Vec3::new(1.0, 0.0, 1.0)),
        Knot::at(Vec3::new(2.0, 0.0, -1.0)),
        Knot::at(Vec3::new(3.0, 0.0, 0.0)),
    ];
    let mut previous = eval::up(&curved, false, 0.0);
    for step in 1..=40 {
        let up = eval::up(&curved, false, step as f32 / 40.0);
        assert!(
            up.dot(previous) > 0.0,
            "up flipped at {step}: {previous} -> {up}"
        );
        previous = up;
    }
}

/// The frame is a frame: square to the heading, everywhere.
#[test]
fn the_up_stays_square() {
    let curved = vec![
        Knot::at(Vec3::ZERO),
        Knot::at(Vec3::new(2.0, 1.0, 0.0)),
        Knot::at(Vec3::new(4.0, 0.0, 2.0)),
    ];
    for step in 0..=20 {
        let t = step as f32 / 20.0;
        let (up, heading) = (
            eval::up(&curved, false, t),
            eval::tangent(&curved, false, t),
        );
        assert!(up.dot(heading).abs() < 1e-3, "not square at {t}");
        assert!((up.length() - 1.0).abs() < 1e-3, "not unit at {t}");
    }
}

/// What a rail and spline gravity ask every frame.
#[test]
fn the_nearest_point_is_found() {
    let points = straight();
    let found = nearest::nearest_point(&points, false, Vec3::new(1.5, 5.0, 0.0));
    assert!(found.distance(Vec3::new(1.5, 0.0, 0.0)) < 1e-2, "{found}");
}

/// An empty or single-knot spline is authored constantly — the first click makes one — and must
/// answer rather than panic or hand back NaN.
#[test]
fn a_spline_too_short_is_safe() {
    for points in [vec![], vec![Knot::at(Vec3::X)]] {
        let closed = false;
        assert!(eval::at(&points, closed, 0.5).is_finite());
        assert!(eval::up(&points, closed, 0.5).is_finite());
        assert_eq!(arc::length(&points, closed), 0.0);
        assert!(arc::at_distance(&points, closed, 1.0).is_finite());
        assert!(nearest::nearest_point(&points, closed, Vec3::ONE).is_finite());
    }
}

/// Coincident knots have no length to divide by, and the guard is `!(total > 0.0)`.
#[test]
fn a_zero_length_spline_is_safe() {
    let stalled = vec![Knot::at(Vec3::ZERO), Knot::at(Vec3::ZERO)];
    assert_eq!(arc::length(&stalled, false), 0.0);
    assert!(arc::at_distance(&stalled, false, 1.0).is_finite());
    assert!(eval::up(&stalled, false, 0.5).is_finite());
}

/// The three modes have to differ, or authoring a tangent does nothing and the field is a lie.
///
/// 🔴 Read on the FIRST segment, where the middle knot contributes its `arriving` — on the second it
/// contributes `leaving` alone and `Aligned` and `Broken` cannot differ. And `arriving` is not the
/// mirror of `leaving` here: a mirror is what `Aligned` means, so mirroring it would compare a mode
/// against itself and prove nothing.
#[test]
fn the_tangent_modes_differ() {
    let bent = |mode: u32| {
        vec![
            Knot::at(Vec3::ZERO),
            Knot {
                position: Vec3::new(1.0, 0.0, 0.0),
                mode,
                leaving: Vec3::new(0.0, 2.0, 0.0),
                arriving: Vec3::new(-1.0, -1.0, 0.0),
                ..Knot::default()
            },
            Knot::at(Vec3::new(2.0, 0.0, 0.0)),
        ]
    };
    let sample = |points: &[Knot]| eval::at(points, false, 0.25);

    let auto = sample(&bent(TANGENT_AUTO));
    let aligned = sample(&bent(TANGENT_ALIGNED));
    let broken = sample(&bent(TANGENT_BROKEN));

    // Auto derives from the neighbours and ignores both authored handles.
    assert!(
        auto.distance(aligned) > 0.1,
        "auto={auto} aligned={aligned}"
    );
    // Broken reads `arriving` on its own, so it arrives differently from the mirrored case.
    assert!(
        broken.distance(aligned) > 0.1,
        "broken={broken} aligned={aligned}"
    );
    assert!(broken.distance(auto) > 0.1, "broken={broken} auto={auto}");
}

/// 🔴 Broken is the only mode that can put a corner in the curve — the others are smooth through the
/// knot by construction. A rail or a swept mesh across a corner is a different thing to handle.
#[test]
fn only_broken_makes_a_corner() {
    let corner = |mode: u32| {
        vec![
            Knot::at(Vec3::ZERO),
            Knot {
                position: Vec3::new(1.0, 0.0, 0.0),
                mode,
                leaving: Vec3::new(0.0, 1.0, 0.0),
                arriving: Vec3::new(1.0, 0.0, 0.0),
                ..Knot::default()
            },
            Knot::at(Vec3::new(2.0, 0.0, 0.0)),
        ]
    };
    // Either side of the middle knot, which sits at t = 0.5 of two segments.
    let across = |points: &[Knot]| {
        eval::tangent(points, false, 0.49).dot(eval::tangent(points, false, 0.51))
    };
    assert!(across(&corner(TANGENT_ALIGNED)) > 0.99, "aligned bent");
    assert!(
        across(&corner(TANGENT_BROKEN)) < 0.9,
        "broken stayed smooth"
    );
}

/// Closed adds the segment back to the first knot, which is what a circuit of rails needs.
#[test]
fn closing_adds_a_segment() {
    let triangle = vec![
        Knot::at(Vec3::ZERO),
        Knot::at(Vec3::new(1.0, 0.0, 0.0)),
        Knot::at(Vec3::new(0.5, 0.0, 1.0)),
    ];
    assert_eq!(eval::segments(&triangle, false), 2);
    assert_eq!(eval::segments(&triangle, true), 3);

    // The closing segment ends where the curve started, so there is no seam to fall through.
    let ended = eval::at(&triangle, true, 1.0);
    assert!(ended.distance(Vec3::ZERO) < 1e-3, "{ended}");
    assert!(arc::length(&triangle, true) > arc::length(&triangle, false));
}

use super::*;
use crate::Adjacency;
use crate::sweep::PROFILE_STEPS;
use kooch_ecs::spline::DEFAULT_RADIUS;

fn flight() -> SplineSweep {
    SplineSweep {
        profile: PROFILE_STEPS,
        spans: 4,
        drop: 0.2,
        ..SplineSweep::default()
    }
}

/// A curve climbing +Y as it runs +Z: the plain straight flight.
fn climbing() -> Vec<Knot> {
    [Vec3::ZERO, Vec3::new(0.0, 2.0, 4.0)]
        .map(Knot::at)
        .to_vec()
}

/// 🔴 The one a human cannot see by looking. Steps are separate boxes on purpose, and a tread
/// welded to the riser behind it is an edge four faces meet at — `Adjacency` reads that as a leak
/// and the face tool selects two steps as one surface.
#[test]
fn a_flight_is_closed() {
    let mesh = steps(&climbing(), false, &flight());
    assert!(mesh.face_count() > 0);
    assert!(Adjacency::of(&mesh).is_closed(), "the flight leaks");
}

/// The rise comes from the curve, which is what makes straight, curved and spiral one generator.
#[test]
fn a_climb_becomes_risers() {
    let mesh = steps(&climbing(), false, &flight());
    let mut heights: Vec<f32> = mesh.positions().iter().map(|point| point.y).collect();
    heights.sort_by(f32::total_cmp);
    let (low, high) = (heights[0], heights[heights.len() - 1]);
    assert!((high - 2.0).abs() < 0.05, "the top tread is at {high}");
    // Down to the curve's own start: a flight rests on the ground rather than floating over it.
    assert!(low.abs() < 0.05, "the bottom reaches {low}");
}

/// A flat curve is a catwalk, not a staircase — and the same code has to make it.
#[test]
fn a_flat_curve_is_level() {
    let flat = [Vec3::ZERO, Vec3::new(0.0, 0.0, 4.0)]
        .map(Knot::at)
        .to_vec();
    let mesh = steps(&flat, false, &flight());
    assert!(Adjacency::of(&mesh).is_closed());
    assert!(
        mesh.positions().iter().all(|point| point.y <= 1.0e-4),
        "a level curve lifted a tread above its own height",
    );
}

/// 🔴 Even treads by ARC LENGTH. Spacing by `t` bunches the steps where the curve is tight, which
/// is the one thing a stair may not do and the one thing no test but this would catch.
#[test]
fn treads_are_evenly_spaced() {
    // Two segments of very different length, so `t` and distance disagree sharply.
    let uneven = [
        Vec3::ZERO,
        Vec3::new(0.0, 0.0, 10.0),
        Vec3::new(0.0, 0.0, 12.0),
    ]
    .map(Knot::at)
    .to_vec();
    let mesh = steps(&uneven, false, &flight());
    let mut edges: Vec<f32> = mesh
        .positions()
        .iter()
        .map(|point| point.z)
        .filter(|z| *z > 1.0e-3)
        .collect();
    edges.sort_by(f32::total_cmp);
    edges.dedup_by(|a, b| (*a - *b).abs() < 1.0e-3);
    let gaps: Vec<f32> = edges.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let (tight, wide) = gaps.iter().fold((f32::MAX, 0.0_f32), |(low, high), gap| {
        (low.min(*gap), high.max(*gap))
    });
    assert!(wide / tight < 1.5, "treads run {tight} to {wide} apart");
}

/// The knot's radius is the tread's width, so the taper that narrows a pipe narrows a flight.
#[test]
fn a_radius_widens_the_tread() {
    let mut points = climbing();
    points[1].radius = DEFAULT_RADIUS * 4.0;
    let mesh = steps(&points, false, &flight());
    let widest = mesh
        .positions()
        .iter()
        .fold(0.0_f32, |far, point| far.max(point.x.abs()));
    assert!((widest - 2.0).abs() < 0.1, "the flight reaches {widest}");
}

/// States an author passes through while drawing. A flight of nothing is better than a flight of
/// degenerate faces the face tool then offers to extrude.
#[test]
fn a_degenerate_flight_is_empty() {
    assert_eq!(steps(&[], false, &flight()).face_count(), 0);
    assert_eq!(steps(&climbing()[..1], false, &flight()).face_count(), 0);
    let stalled = vec![Knot::at(Vec3::ZERO); 3];
    assert_eq!(steps(&stalled, false, &flight()).face_count(), 0);
}

/// Steps reach `sweep` through the same door as every other profile, or the editor's sync — which
/// only ever calls `sweep` — builds a pipe where the author asked for stairs.
#[test]
fn the_profile_picks_steps() {
    let mesh = crate::sweep(&climbing(), false, &flight());
    assert_eq!(
        mesh.face_count(),
        steps(&climbing(), false, &flight()).face_count()
    );
}

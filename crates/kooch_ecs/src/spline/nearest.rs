//! The point on a [`Spline`](super::Spline) closest to somewhere else.
//!
//! What an auto-dolly asks to put a camera beside whoever it follows (#1262), and what gravity along
//! a path asks to know which way is down (#1429) — both every frame, for every body.
//!
//! ⚠️ **The answer is not always unique, and that is geometry rather than a bug.** Inside a curve's
//! centre of curvature, and along the axis of a closed loop, two stretches of the curve are
//! equidistant: a body crossing that surface — the medial axis — sees the answer jump from one to
//! the other. Nothing here can smooth it, because both answers are correct. A consumer that cannot
//! take the jump has to keep its field away from that surface or blend two sources across it.

use glam::Vec3;

use super::Knot;
use super::eval::{at, segments};

/// The `t` of the closest point on the curve to `point`.
///
/// Coarse samples to find which stretch wins, then a shrinking search inside it. Sampling alone
/// would quantise the answer to the sample spacing, which a camera reads as a stair.
pub fn nearest(points: &[Knot], closed: bool, point: Vec3) -> f32 {
    let count = segments(points, closed);
    if count == 0 {
        return 0.0;
    }
    let steps = count * SAMPLES_PER_SEGMENT as usize;
    let coarse = coarsest(points, closed, point, steps);

    // The winner's neighbours bound the true closest: the distance along a curve has no second
    // minimum inside one sample's span, so refining this window cannot walk away from it.
    let span = 1.0 / steps as f32;
    refined(
        points,
        closed,
        point,
        (coarse - span).max(0.0),
        (coarse + span).min(1.0),
    )
}

/// The closest point itself, for the common case that never needed the parameter.
pub fn nearest_point(points: &[Knot], closed: bool, point: Vec3) -> Vec3 {
    at(points, closed, nearest(points, closed, point))
}

/// The sampled `t` whose point sits closest.
fn coarsest(points: &[Knot], closed: bool, point: Vec3, steps: usize) -> f32 {
    let mut best = (0.0_f32, f32::INFINITY);
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let distance = at(points, closed, t).distance_squared(point);
        if distance < best.1 {
            best = (t, distance);
        }
    }
    best.0
}

/// Golden-section search over the window, which needs no derivative — and the derivative of a
/// distance to a cubic is a quintic whose roots are their own problem.
fn refined(points: &[Knot], closed: bool, point: Vec3, low: f32, high: f32) -> f32 {
    let measure = |t: f32| at(points, closed, t).distance_squared(point);
    let (mut low, mut high) = (low, high);
    let mut inner = (high - (high - low) / GOLDEN, low + (high - low) / GOLDEN);
    let mut measured = (measure(inner.0), measure(inner.1));

    for _ in 0..REFINEMENTS {
        if measured.0 < measured.1 {
            high = inner.1;
            inner.1 = inner.0;
            measured.1 = measured.0;
            inner.0 = high - (high - low) / GOLDEN;
            measured.0 = measure(inner.0);
        } else {
            low = inner.0;
            inner.0 = inner.1;
            measured.0 = measured.1;
            inner.1 = low + (high - low) / GOLDEN;
            measured.1 = measure(inner.1);
        }
    }
    (low + high) * 0.5
}

/// Coarse samples per segment, to pick the stretch before refining inside it.
const SAMPLES_PER_SEGMENT: u32 = 8;

/// Each one shrinks the window to 62 % of itself, so 24 takes a segment's eighth below a micrometre.
const REFINEMENTS: u32 = 24;

const GOLDEN: f32 = 1.618_034;

//! How long a [`Spline`](super::Spline) is, and where it is at a distance.
//!
//! 🔴 This is the half that `t` cannot answer. Equal steps of `t` are **not** equal steps along the
//! path — a tight segment and a long one each take the same share of the parameter — so anything
//! that spaces, drives or paces along a curve measures in metres here, not in `t`.
//!
//! Spacing collectibles evenly (#1428) and a rail at a fixed speed (#1262) both read
//! [`at_distance`]; without it they would bunch up wherever the curve bends.

use glam::Vec3;

use super::Knot;
use super::eval::{at, segments};

/// How long the curve is, in the entity's units.
pub fn length(points: &[Knot], closed: bool) -> f32 {
    walked(points, closed).last().copied().unwrap_or(0.0)
}

/// Where the curve is `distance` along it, clamped to its ends.
///
/// Inverts the arc-length table by searching it, then interpolates inside the step it lands in. The
/// error is the chord against the arc of one step, which is why [`STEPS_PER_SEGMENT`] is the number
/// that decides the accuracy.
pub fn at_distance(points: &[Knot], closed: bool, distance: f32) -> Vec3 {
    let table = walked(points, closed);
    match parameter(&table, distance) {
        Some(t) => at(points, closed, t),
        None => points.first().map_or(Vec3::ZERO, |knot| knot.position),
    }
}

/// The `t` that lands `distance` along the curve, for a caller that wants the tangent or the up
/// there too and would otherwise pay for the table twice.
pub fn parameter_at(points: &[Knot], closed: bool, distance: f32) -> f32 {
    parameter(&walked(points, closed), distance).unwrap_or(0.0)
}

/// Distances travelled at each sample, starting at zero. Empty where there is no curve.
///
/// ⚠️ Rebuilt on every call. A spline is authored once and read many times, so this is the thing to
/// cache first — but against a measurement, not a hunch, and a cache has to be invalidated by an
/// edit that the editor is free to make at any time.
pub fn walked(points: &[Knot], closed: bool) -> Vec<f32> {
    let count = segments(points, closed);
    if count == 0 {
        return Vec::new();
    }
    let steps = count * STEPS_PER_SEGMENT as usize;
    let mut table = Vec::with_capacity(steps + 1);
    table.push(0.0);

    let mut travelled = 0.0;
    let mut previous = at(points, closed, 0.0);
    for step in 1..=steps {
        let sampled = at(points, closed, step as f32 / steps as f32);
        travelled += previous.distance(sampled);
        table.push(travelled);
        previous = sampled;
    }
    table
}

/// The `t` for a distance, given the table. `None` where the table has nothing to search.
fn parameter(table: &[f32], distance: f32) -> Option<f32> {
    let total = *table.last()?;
    let steps = table.len() - 1;
    if steps == 0 {
        return Some(0.0);
    }
    // 🔴 `!(total > 0.0)`, not `total <= 0.0`: every comparison against NaN is false, and a curve
    // whose points coincide has no length to divide by.
    if !(total > 0.0) {
        return Some(0.0);
    }
    let wanted = distance.clamp(0.0, total);

    // The table only grows, so the first sample at or past `wanted` bounds the step it falls in.
    let found = table.partition_point(|&walked| walked < wanted).max(1);
    let (before, after) = (table[found - 1], table[found]);
    let span = after - before;
    let inside = match span > 0.0 {
        true => (wanted - before) / span,
        // A step that covered no ground: the curve stalls here and either end of it is the answer.
        false => 0.0,
    };
    Some(((found - 1) as f32 + inside) / steps as f32)
}

/// Samples each segment is walked in to measure it. The error of a chord against its arc falls with
/// the square of this, so 16 puts a 90° turn of a one-metre segment inside a millimetre.
const STEPS_PER_SEGMENT: u32 = 16;

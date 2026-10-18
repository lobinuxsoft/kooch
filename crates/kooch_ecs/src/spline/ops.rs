//! Operations that take a [`Spline`](super::Spline) and give back another one (#1447).
//!
//! Everything in [`eval`](super::eval), [`arc`](super::arc) and [`nearest`](super::nearest) *reads*
//! a curve. These **return** one, which is what makes a curve authorable rather than only
//! sampleable: #1506 scatters on [`resample`], #1509 runs meshes on it, and #1526 extrudes a face
//! along it.
//!
//! 🔴 Every one is pure — `&[Knot] -> Vec<Knot>` — and none of them touches the world, the GPU or
//! the wire. A caller that wants to edit a component does it by emitting the whole list, the way
//! the Inspector's list widget already does, so there is one way a curve gets written.
//!
//! ⚠️ The knots a resample or a subdivide produces are `TANGENT_AUTO`. They sit **on** the curve
//! they came from, and Catmull-Rom through points on a curve tracks it closely — but it is not the
//! same curve. Anything that needs the original shape exactly should sample it, not rebuild it.

use glam::Vec3;

use super::eval::{radius_at, roll_at, segments};
use super::{Knot, TANGENT_AUTO, arc, eval};

/// Knots spaced evenly **by arc length**, replacing the list.
///
/// 🔴 By distance, never by `t`. A tight segment and a long one take the same share of the
/// parameter, so spacing by `t` bunches points up wherever the curve bends — which is exactly
/// where an extrude or a scatter must not bunch.
///
/// A closed curve gets `count` knots and stays closed: the last one does not repeat the first,
/// because the closing segment is already a segment.
pub fn resample(points: &[Knot], closed: bool, count: usize) -> Vec<Knot> {
    if segments(points, closed) == 0 || count < 2 {
        return points.to_vec();
    }
    let length = arc::length(points, closed);
    if !(length > 0.0) {
        return points.to_vec();
    }
    // An open curve puts a knot on each end, so the gaps are one fewer than the knots. A closed one
    // has as many gaps as knots, the last running back to the first.
    let gaps = match closed {
        true => count,
        false => count - 1,
    };
    (0..count)
        .map(|index| sampled(points, closed, length * index as f32 / gaps as f32))
        .collect()
}

/// Extra knots inside every segment, at even `t` within each.
///
/// Per segment and by parameter, which is what subdivision means: it adds detail where the author
/// put points, leaving the distribution they chose alone. [`resample`] is the one that evens out
/// spacing, and they are different requests.
pub fn subdivide(points: &[Knot], closed: bool, cuts: usize) -> Vec<Knot> {
    let spans = segments(points, closed);
    if spans == 0 || cuts == 0 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(points.len() + spans * cuts);
    for span in 0..spans {
        out.push(points[span]);
        for cut in 1..=cuts {
            let local = cut as f32 / (cuts + 1) as f32;
            out.push(at_parameter(
                points,
                closed,
                (span as f32 + local) / spans as f32,
            ));
        }
    }
    // The closing segment ends at the first knot, which is already at the front of the list.
    if !closed {
        out.push(points[points.len() - 1]);
    }
    out
}

/// The curve walked the other way.
///
/// Takes no `closed`: reversing a ring gives the same ring walked backwards, and the closing
/// segment still runs from the last knot to the first.
///
/// 🔴 The roll is negated. It is measured about the tangent, and the tangent now points the other
/// way, so keeping the number would bank the curve the wrong side. The handles only swap: both are
/// authored pointing **away** from their knot, so neither changes direction — only which of the two
/// the curve arrives on.
pub fn reverse(points: &[Knot]) -> Vec<Knot> {
    points
        .iter()
        .rev()
        .map(|knot| Knot {
            arriving: knot.leaving,
            leaving: knot.arriving,
            roll: -knot.roll,
            ..*knot
        })
        .collect()
}

/// The curve between two distances along it, in the entity's units.
///
/// 🔴 The result is **open**, always. A ring cut anywhere is a line, and reporting it as closed
/// would join the two fresh ends back together — the one part of the original the author just
/// asked to remove.
///
/// The knots inside the range are kept as they were authored, so a trim does not re-derive the
/// shape it is preserving; only the two ends are new.
pub fn trim(points: &[Knot], closed: bool, from: f32, to: f32) -> Vec<Knot> {
    if segments(points, closed) == 0 {
        return points.to_vec();
    }
    let length = arc::length(points, closed);
    if !(length > 0.0) {
        return points.to_vec();
    }
    let (near, far) = match from <= to {
        true => (from, to),
        false => (to, from),
    };
    let (near, far) = (near.clamp(0.0, length), far.clamp(0.0, length));
    // A zero-width range has no curve in it, and two knots on the same spot is not one either.
    if !(far - near > f32::EPSILON) {
        return Vec::new();
    }

    let mut out = vec![sampled(points, closed, near)];
    out.extend(
        distances(points, closed)
            .into_iter()
            .enumerate()
            .filter(|(_, reached)| *reached > near && *reached < far)
            .map(|(index, _)| points[index]),
    );
    out.push(sampled(points, closed, far));
    out
}

/// Corners rounded by `radius`, each becoming a pair of knots the curve bends between.
///
/// Pulls the corner back along both of its neighbours instead of fitting a true circular arc: two
/// points on the lines into the corner, with the curve's own interpolation rounding between them.
/// A fitted arc is exact and needs its own tangents; this is what a blockout wants, and it is the
/// operation that turns a `Broken` knot into a bend.
///
/// 🔴 Clamped to half of each neighbouring segment, so two corners filleted from one short segment
/// cannot cross each other and fold the curve back on itself.
pub fn fillet(points: &[Knot], closed: bool, radius: f32) -> Vec<Knot> {
    let count = points.len();
    if count < 3 || !(radius > 0.0) {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(count * 2);
    for index in 0..count {
        // An open curve's ends have one neighbour, so there is no corner there to round.
        let corner = closed || (index > 0 && index + 1 < count);
        if !corner {
            out.push(points[index]);
            continue;
        }
        let here = points[index].position;
        let back = points[(index + count - 1) % count].position;
        let ahead = points[(index + 1) % count].position;
        match (pulled(here, back, radius), pulled(here, ahead, radius)) {
            (Some(before), Some(after)) => {
                out.push(Knot {
                    position: before,
                    mode: TANGENT_AUTO,
                    ..points[index]
                });
                out.push(Knot {
                    position: after,
                    mode: TANGENT_AUTO,
                    ..points[index]
                });
            }
            // A neighbour sitting on top of the corner gives no direction to pull along, and a
            // rounded corner nobody can aim at is worse than the corner.
            _ => out.push(points[index]),
        }
    }
    out
}

/// A point `radius` from `here` towards `neighbour`, never past the halfway mark.
fn pulled(here: Vec3, neighbour: Vec3, radius: f32) -> Option<Vec3> {
    let gap = neighbour - here;
    let span = gap.length();
    // 🔴 `!(span > 0.0)`, not `span <= 0.0`: a NaN position would pass the `<=` and divide into a
    // knot at NaN, which is a curve nothing downstream can draw or report on.
    if !(span > 0.0) {
        return None;
    }
    Some(here + gap / span * radius.min(span / 2.0))
}

/// A knot on the curve `distance` along it, carrying the roll and radius authored there.
fn sampled(points: &[Knot], closed: bool, distance: f32) -> Knot {
    at_parameter(points, closed, arc::parameter_at(points, closed, distance))
}

/// A knot on the curve at `t`, carrying the roll and radius authored there.
fn at_parameter(points: &[Knot], closed: bool, t: f32) -> Knot {
    Knot {
        position: eval::at(points, closed, t),
        mode: TANGENT_AUTO,
        arriving: Vec3::ZERO,
        leaving: Vec3::ZERO,
        roll: roll_at(points, closed, t),
        radius: radius_at(points, closed, t),
    }
}

/// How far along the curve each authored knot sits. Indexed the same as `points`.
fn distances(points: &[Knot], closed: bool) -> Vec<f32> {
    let table = arc::walked(points, closed);
    let step = arc::STEPS_PER_SEGMENT as usize;
    (0..points.len())
        .map(|index| table.get(index * step).copied().unwrap_or(0.0))
        .collect()
}

#[cfg(test)]
mod tests;

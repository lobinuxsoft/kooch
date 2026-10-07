//! Where a [`Spline`](super::Spline) passes, which way it heads, and which way is up there.
//!
//! `t` runs 0..1 over the whole curve and is **uniform in parameter, not in distance** — equal steps
//! of `t` are not equal steps along the path. Anything that spaces things evenly wants
//! [`arc`](super::arc) instead.

use glam::Vec3;

use super::{Knot, TANGENT_AUTO, TANGENT_BROKEN};

/// How many segments the points make: one per gap, plus the closing one.
pub fn segments(points: &[Knot], closed: bool) -> usize {
    match points.len() {
        0 | 1 => 0,
        len if closed => len,
        len => len - 1,
    }
}

/// Where the curve passes at `t`, clamped to its ends.
pub fn at(points: &[Knot], closed: bool, t: f32) -> Vec3 {
    let Some((a, b, local)) = segment_at(points, closed, t) else {
        return points.first().map_or(Vec3::ZERO, |knot| knot.position);
    };
    let (ma, mb) = tangents(points, closed, a, b);
    hermite(points[a].position, ma, points[b].position, mb, local)
}

/// Which way the curve heads at `t`, normalised. Zero-length where the curve does not move.
pub fn tangent(points: &[Knot], closed: bool, t: f32) -> Vec3 {
    let Some((a, b, local)) = segment_at(points, closed, t) else {
        return Vec3::ZERO;
    };
    let (ma, mb) = tangents(points, closed, a, b);
    let derivative = hermite_slope(points[a].position, ma, points[b].position, mb, local);
    // A cusp — both tangents zero against a zero gap — has no direction to report, and normalising
    // it would hand back NaN to a caller that only asked where it was going.
    match derivative.length_squared() > 1e-12 {
        true => derivative.normalize(),
        false => Vec3::ZERO,
    }
}

/// Which way is up at `t`: the rotation-minimising frame, rolled by the knots' `roll`.
///
/// 🔴 **Parallel transport, never Frenet.** The Frenet normal is the curvature direction, and it
/// **flips through an inflection** where curvature passes zero — a visual glitch for a swept mesh
/// (#1152) and, where this up *is* which way is down, a character thrown off the road in one frame
/// (#1429). Transport carries the previous up forward instead, so it has no opinion to flip.
///
/// ⚠️ Costs `RMF_STEPS` samples because transport has no closed form: the frame at `t` is the
/// history of every frame before it. Closed curves need not come back to the up they started with —
/// that residue is the curve's holonomy and is geometry, not a bug.
pub fn up(points: &[Knot], closed: bool, t: f32) -> Vec3 {
    let start = tangent(points, closed, 0.0);
    if start.length_squared() < 1e-12 {
        return Vec3::Y;
    }
    let mut carried = seeded(start);
    let mut previous = (at(points, closed, 0.0), start);
    let reached = t.clamp(0.0, 1.0);

    for step in 1..=RMF_STEPS {
        let sampled = reached * step as f32 / RMF_STEPS as f32;
        let (position, heading) = (
            at(points, closed, sampled),
            tangent(points, closed, sampled),
        );
        if heading.length_squared() > 1e-12 {
            carried = reflected(carried, previous, (position, heading));
            previous = (position, heading);
        }
    }
    rolled(carried, previous.1, roll_at(points, closed, reached))
}

/// Samples the transport takes to reach any `t`. Enough that a hairpin does not out-turn it, few
/// enough to stay cheap; a cached table is an optimisation to make against a measurement, not now.
const RMF_STEPS: u32 = 32;

/// One step of Wang's double-reflection method: reflect the frame through the plane between the two
/// samples, then through the second's own tangent plane. Two reflections preserve length and leave
/// the frame with no twist of its own.
fn reflected(up: Vec3, from: (Vec3, Vec3), to: (Vec3, Vec3)) -> Vec3 {
    let gap = to.0 - from.0;
    let once = match gap.length_squared() > 1e-12 {
        true => mirror(up, gap),
        false => up,
    };
    let heading = match gap.length_squared() > 1e-12 {
        true => mirror(from.1, gap),
        false => from.1,
    };
    let twice = mirror(once, to.1 - heading);
    orthogonal(twice, to.1)
}

/// Reflects `v` through the plane whose normal is `axis`. An axis too short to have a direction
/// leaves `v` alone rather than returning NaN.
fn mirror(v: Vec3, axis: Vec3) -> Vec3 {
    let squared = axis.length_squared();
    match squared > 1e-12 {
        true => v - axis * (2.0 * v.dot(axis) / squared),
        false => v,
    }
}

/// The part of `up` square to `heading`, renormalised. Transport drifts off perpendicular by
/// floating-point residue over 32 steps, and a frame that is not square is not a frame.
fn orthogonal(up: Vec3, heading: Vec3) -> Vec3 {
    let flattened = up - heading * up.dot(heading);
    match flattened.length_squared() > 1e-12 {
        true => flattened.normalize(),
        false => seeded(heading),
    }
}

/// A first up for a tangent with no history: world up, unless the curve heads along it.
pub(crate) fn seeded(heading: Vec3) -> Vec3 {
    let flattened = Vec3::Y - heading * Vec3::Y.dot(heading);
    match flattened.length_squared() > 1e-6 {
        true => flattened.normalize(),
        // Straight up or straight down: no horizon to pick, so any square axis will do.
        false => heading.cross(Vec3::X).normalize_or(Vec3::Z),
    }
}

/// Turns `up` about the heading by `degrees`.
fn rolled(up: Vec3, heading: Vec3, degrees: f32) -> Vec3 {
    if degrees == 0.0 || heading.length_squared() < 1e-12 {
        return up;
    }
    glam::Quat::from_axis_angle(heading.normalize(), degrees.to_radians()) * up
}

/// The roll at `t`, straight between the two knots of the segment.
fn roll_at(points: &[Knot], closed: bool, t: f32) -> f32 {
    let Some((a, b, local)) = segment_at(points, closed, t) else {
        return points.first().map_or(0.0, |knot| knot.roll);
    };
    points[a].roll + (points[b].roll - points[a].roll) * local
}

/// Which segment `t` lands in, and where inside it: `(first knot, second knot, 0..1)`.
pub(crate) fn segment_at(points: &[Knot], closed: bool, t: f32) -> Option<(usize, usize, f32)> {
    let count = segments(points, closed);
    if count == 0 {
        return None;
    }
    let scaled = t.clamp(0.0, 1.0) * count as f32;
    // The last segment owns `t == 1`: flooring it would index one past the end.
    let index = (scaled.floor() as usize).min(count - 1);
    Some((index, (index + 1) % points.len(), scaled - index as f32))
}

/// The Hermite tangents of the segment from knot `a` to knot `b`.
///
/// 🔴 Authored handles point **away** from their knot, as every editor draws them, so the tangent
/// arriving at `b` is `-b.arriving`. `Aligned` needs no mirroring here for the same reason: two
/// handles pointing opposite ways already are one straight line through the point.
///
/// Public because a gizmo draws these: a handle derived a second time in the editor is a second
/// answer, free to disagree with the curve it is drawn over (#1387's lesson, on another axis).
pub fn tangents(points: &[Knot], closed: bool, a: usize, b: usize) -> (Vec3, Vec3) {
    let leaving = match points[a].mode {
        TANGENT_AUTO => auto(points, closed, a),
        _ => points[a].leaving,
    };
    let arriving = match points[b].mode {
        TANGENT_AUTO => auto(points, closed, b),
        TANGENT_BROKEN => -points[b].arriving,
        // Aligned authors `leaving` alone and mirrors it, so `-arriving` is `leaving` again. Storing
        // the mirror would be a second copy of one direction, free to disagree with itself.
        _ => points[b].leaving,
    };
    (leaving, arriving)
}

/// Catmull-Rom: half the span between the neighbours. An end with no neighbour leans on itself, so
/// the curve leaves it heading at the next point rather than stalling.
fn auto(points: &[Knot], closed: bool, index: usize) -> Vec3 {
    let len = points.len();
    let (before, after) = match closed {
        true => ((index + len - 1) % len, (index + 1) % len),
        false => (index.saturating_sub(1), (index + 1).min(len - 1)),
    };
    (points[after].position - points[before].position) * 0.5
}

/// Cubic Hermite at `t` in 0..1.
fn hermite(p0: Vec3, m0: Vec3, p1: Vec3, m1: Vec3, t: f32) -> Vec3 {
    let (t2, t3) = (t * t, t * t * t);
    p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
        + m0 * (t3 - 2.0 * t2 + t)
        + p1 * (-2.0 * t3 + 3.0 * t2)
        + m1 * (t3 - t2)
}

/// Its derivative, which is the unnormalised tangent.
fn hermite_slope(p0: Vec3, m0: Vec3, p1: Vec3, m1: Vec3, t: f32) -> Vec3 {
    let t2 = t * t;
    p0 * (6.0 * t2 - 6.0 * t)
        + m0 * (3.0 * t2 - 4.0 * t + 1.0)
        + p1 * (-6.0 * t2 + 6.0 * t)
        + m1 * (3.0 * t2 - 2.0 * t)
}

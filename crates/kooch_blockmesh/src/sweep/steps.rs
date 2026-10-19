//! Steps placed along a [`Spline`](kooch_ecs::spline::Spline) — the second half of #1152.
//!
//! 🔴 **Straight, curved and spiral flights are one generator.** The curve carries the plan and the
//! climb; a flat curve gives a catwalk, a climbing one a flight, a climbing circle a spiral. There
//! is nothing here that reads which of those it is.
//!
//! 🔴 **Treads are level, whatever the curve's roll.** A knot's roll banks a pipe; a banked step is
//! one nobody can stand on. The curve decides where a step is and which way it faces, never how
//! far it leans.

use glam::Vec3;

use kooch_ecs::spline::{Knot, arc, eval};

use super::{MAX_RINGS, MIN_RADIUS, SplineSweep};
use crate::BlockMesh;
use crate::shapes::Builder;

/// Thinnest tread, so a `drop` dragged to zero cannot collapse a step into a sheet.
const MIN_DROP: f32 = 1.0e-3;

/// Shortest step the curve can advance and still be a step rather than a seam.
const MIN_RUN: f32 = 1.0e-3;

/// The flight the curve describes, as an authoring mesh.
///
/// Each step is its own closed box, welded to nothing: a riser shared between two steps is an edge
/// four faces meet at, which `Adjacency::of` reads as a leak and face selection reads as one
/// surface spanning two steps.
pub fn steps(points: &[Knot], closed: bool, of: &SplineSweep) -> BlockMesh {
    let segments = eval::segments(points, closed);
    if segments == 0 {
        return BlockMesh::default();
    }
    let count = (of.spans.max(1) as usize * segments).min(MAX_RINGS);
    let length = arc::length(points, closed);
    // 🔴 `!(x > 0.0)`: a NaN length divides every tread into NaN, and a mesh of NaN draws as
    // nothing with no error anywhere to say why.
    if !(length > 0.0) {
        return BlockMesh::default();
    }

    // Walked once. Stepping by `t` instead would bunch the treads where the curve is tight, which
    // is the one thing a stair may not do.
    let table = arc::walked(points, closed);
    let walk = |index: usize| arc::parameter_in(&table, length * index as f32 / count as f32);
    let drop = of.drop.max(MIN_DROP);

    let mut out = Builder::default();
    for index in 0..count {
        let (here, ahead) = (walk(index), walk(index + 1));
        // The tread sits at the height the curve reaches at its far edge, so a flight ends level
        // with the knot the author put there rather than one riser below it.
        let top = eval::at(points, closed, ahead).y;
        // 🔴 Down to the step before it, and `drop` is only the floor on that. A step that stopped
        // at its own thickness would hang in the air over the one below, which reads as a flight
        // of floating slabs — a style, not a staircase, and never what a blockout wants first.
        let under = eval::at(points, closed, here).y.min(top - drop);
        if let Some(step) = tread(points, closed, (here, ahead), (top, under)) {
            out.append(step);
        }
    }
    out.finish()
}

/// One step as a box of its own, or `None` where the curve gives it no area.
fn tread(points: &[Knot], closed: bool, span: (f32, f32), heights: (f32, f32)) -> Option<Builder> {
    let (here, ahead) = span;
    let (top, floor) = heights;
    let (back, front) = (
        level(points, closed, here, top),
        level(points, closed, ahead, top),
    );
    if back.distance(front) < MIN_RUN {
        return None;
    }
    let reach = |at: f32| {
        let radius = eval::radius_at(points, closed, at);
        match radius.is_finite() && radius > MIN_RADIUS {
            true => Some(radius),
            false => None,
        }
    };
    let (wide, tip) = (reach(here)?, reach(ahead)?);
    let (across, along) = (side(points, closed, here), side(points, closed, ahead));

    let corners = [
        back - across * wide,
        back + across * wide,
        front + along * tip,
        front - along * tip,
    ];
    let lower = |point: Vec3| Vec3::new(point.x, floor, point.z);
    let under: Vec<Vec3> = corners.iter().copied().map(lower).collect();
    let middle = (corners.iter().copied().sum::<Vec3>() + under.iter().copied().sum::<Vec3>())
        / (corners.len() as f32 * 2.0);

    let mut step = Builder::default();
    step.face_away(&corners, middle);
    step.face_away(&under, middle);
    for corner in 0..corners.len() {
        let next = (corner + 1) % corners.len();
        step.face_away(
            &[corners[corner], corners[next], under[next], under[corner]],
            middle,
        );
    }
    Some(step)
}

/// Where the curve is at `t`, flattened onto the tread's own height.
fn level(points: &[Knot], closed: bool, t: f32, height: f32) -> Vec3 {
    let at = eval::at(points, closed, t);
    Vec3::new(at.x, height, at.z)
}

/// The horizontal direction across the flight at `t`.
///
/// 🔴 Taken from the tangent flattened, not from `Frame::side`: the frame turns with the curve's
/// roll, and a tread that turns with it is a step tilted sideways. Where the curve runs straight
/// up there is no heading left to flatten, so the frame is what is left to ask.
fn side(points: &[Knot], closed: bool, t: f32) -> Vec3 {
    let tangent = eval::tangent(points, closed, t);
    let flat = Vec3::new(tangent.x, 0.0, tangent.z);
    if flat.length_squared() > 1.0e-12 {
        return flat.normalize().cross(Vec3::Y);
    }
    let frame = eval::frame(points, closed, t);
    let fallback = Vec3::new(frame.side.x, 0.0, frame.side.z);
    fallback.try_normalize().unwrap_or(Vec3::X)
}

#[cfg(test)]
mod tests;

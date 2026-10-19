//! Curves made from parameters rather than from another curve (#1528).
//!
//! 🔴 **Generated once, never live.** Each returns a plain knot list the author then edits by hand
//! like any other. No component remembers it was a circle, so there are no parameters to drift out
//! of step with the knots — a curve the author cannot drag is a curve the author will fight.
//!
//! Built on the **XZ plane around the origin**, in the entity's own space. The transform is how a
//! curve is aimed; a rotation parameter here would be a second way to say the same thing.
//!
//! ⚠️ The knots come out `TANGENT_AUTO`, so these are curves that *pass through* a circle's points
//! rather than true conics: Catmull-Rom bulges outwards between them. Measured against a unit
//! radius (`circle_error_by_segments`): 6 segments is off by **2.6 %**, 8 by 0.85 %, the default 12
//! by **0.17 %**, 24 by 0.011 %. That is what `segments` trades — fidelity against a list short
//! enough to still edit.

use std::f32::consts::TAU;

use glam::Vec3;

use super::Knot;

/// Segments a primitive is built with unless the author says otherwise.
pub const DEFAULT_SEGMENTS: usize = 12;

/// Fewest that enclose or turn anything. Two knots are a line whichever way they are placed.
const MIN_SEGMENTS: usize = 3;

/// Ceiling on the **knots** a primitive produces, so a number typed by mistake cannot produce a
/// list nothing can draw or edit. An open curve carries one more knot than it has segments, which
/// is why the segment count is clamped one below this.
const MAX_KNOTS: usize = 512;

/// A closed ring of `segments` knots, `radius` from the origin.
pub fn circle(radius: f32, segments: usize) -> Vec<Knot> {
    arc(radius, TAU, segments)
}

/// An open arc turning `sweep` radians, `radius` from the origin, starting at +X.
///
/// A full turn gives the same knots as [`circle`], but the caller still has to set `closed` — this
/// returns points, and whether the ends join is the curve's own field.
pub fn arc(radius: f32, sweep: f32, segments: usize) -> Vec<Knot> {
    let segments = segments.clamp(MIN_SEGMENTS, MAX_KNOTS - 1);
    // 🔴 `is_finite` before anything: a NaN radius from a bad field would put NaN in every knot,
    // and a curve of NaN draws nothing with no error anywhere to say why.
    let radius = match radius.is_finite() {
        true => radius.max(0.0),
        false => 0.0,
    };
    let sweep = match sweep.is_finite() {
        true => sweep,
        false => 0.0,
    };
    // A full turn closes on itself, so the last knot would land on the first. One fewer step than
    // a partial sweep, which needs a knot on each end.
    let whole = (sweep.abs() - TAU).abs() < 1.0e-4;
    let count = match whole {
        true => segments,
        false => segments + 1,
    };
    (0..count)
        .map(|step| {
            let angle = sweep * step as f32 / segments as f32;
            Knot::at(Vec3::new(radius * angle.cos(), 0.0, radius * angle.sin()))
        })
        .collect()
}

/// A helix of `turns` turns, climbing `height`, from `start` radius to `end`.
///
/// The path of a spiral staircase, which is the consumer that asked for it (#1152): put a
/// `SplineSweep` with the `Steps` profile on it and the flight follows.
pub fn spiral(start: f32, end: f32, height: f32, turns: f32, segments: usize) -> Vec<Knot> {
    let segments = segments.clamp(MIN_SEGMENTS, MAX_KNOTS - 1);
    let finite = |value: f32, fallback: f32| match value.is_finite() {
        true => value,
        false => fallback,
    };
    let (start, end) = (finite(start, 0.0).max(0.0), finite(end, 0.0).max(0.0));
    let height = finite(height, 0.0);
    // A spiral of no turns is a vertical line, which is a usable answer rather than an error: it
    // is what an author passes through while dragging the number down.
    let turns = finite(turns, 0.0);

    // 🔴 Steps **across the whole spiral**, not per turn. `segments` is the length of the list the
    // author has to edit, and a per-turn count makes that length depend on another field.
    let steps = (segments as f32 * turns.abs().max(1.0)).round() as usize;
    let steps = steps.clamp(MIN_SEGMENTS, MAX_KNOTS - 1);
    (0..=steps)
        .map(|step| {
            let along = step as f32 / steps as f32;
            let angle = TAU * turns * along;
            let radius = start + (end - start) * along;
            Knot::at(Vec3::new(
                radius * angle.cos(),
                height * along,
                radius * angle.sin(),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests;

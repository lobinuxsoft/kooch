//! [`SplineVisualizer`] — the curve, its handles, and which way is up along it (#1261).
//!
//! 🔴 The frame is drawn, not just the curve. A spline is authored for things that ride it: a rail
//! carries a camera's up, a swept profile carries its cross-section, and gravity along a path
//! carries which way is **down** (#1429). A curve with no frame shown looks right while the thing
//! riding it rolls over, and the author has nothing to look at.

use glam::{Mat4, Vec3};

use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::spline::{Knot, Spline, TANGENT_AUTO, arc, eval};
use kooch_gizmos::{Gizmos, Visualizer};

/// The curve itself.
const CURVE: Vec3 = Vec3::new(0.25, 0.8, 1.0);
/// A knot the curve passes through.
const KNOT: Vec3 = Vec3::new(1.0, 1.0, 1.0);
/// An authored tangent handle. Warm, because it is the thing a hand moves.
const HANDLE: Vec3 = Vec3::new(1.0, 0.6, 0.15);
/// A derived tangent, dimmer: `Auto` is not a handle and dragging it does nothing.
const DERIVED: Vec3 = Vec3::new(0.45, 0.35, 0.2);
/// Which way is up there — the axis a rail tilts about and spline gravity calls down.
const UP: Vec3 = Vec3::new(0.4, 0.95, 0.4);

#[derive(Default)]
pub(crate) struct SplineVisualizer;

impl Visualizer<Spline> for SplineVisualizer {
    fn draw(&self, spline: &Spline, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let world = &transform.matrix;
        // One knot is a point with no curve; drawing it is still what tells the author the component
        // is there and took their click.
        if spline.points.len() < 2 {
            if let Some(knot) = spline.points.first() {
                mark(world.transform_point3(knot.position), KNOT, gizmos);
            }
            return;
        }

        let steps = eval::segments(&spline.points, spline.closed) * STEPS_PER_SEGMENT;
        let mut previous = world.transform_point3(eval::at(&spline.points, spline.closed, 0.0));
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let sampled = world.transform_point3(eval::at(&spline.points, spline.closed, t));
            gizmos.line(previous, sampled, CURVE);
            previous = sampled;
        }

        frames(spline, world, gizmos);
        for (index, knot) in spline.points.iter().enumerate() {
            handles(spline, index, knot, world, gizmos);
            mark(world.transform_point3(knot.position), KNOT, gizmos);
        }
    }
}

/// Samples the curve is drawn in, per segment. Enough that a bend reads as a curve rather than a
/// chain of chords.
const STEPS_PER_SEGMENT: usize = 16;

/// Frames drawn along the whole curve. Far fewer than the curve's own samples: each one costs a
/// transport from the start of the curve, so this is the line in this file worth measuring if the
/// editor's frame time ever points here.
const FRAME_TICKS: usize = 8;

/// How long an up tick is drawn, as a share of the curve's length. Proportional, so the frame stays
/// legible on a two-metre curve and on a two-hundred-metre one.
const TICK_SHARE: f32 = 0.04;

/// Which way is up at a few points along the curve.
fn frames(spline: &Spline, world: &Mat4, gizmos: &mut Gizmos<'_>) {
    let length = arc::length(&spline.points, spline.closed);
    // 🔴 `!(length > 0.0)`, not `length <= 0.0`: coincident knots give zero and a NaN would draw a
    // tick to nowhere, which reads as a corrupt curve rather than as a degenerate one.
    if !(length > 0.0) {
        return;
    }
    let tick = length * TICK_SHARE;
    for step in 0..=FRAME_TICKS {
        let t = step as f32 / FRAME_TICKS as f32;
        let at = world.transform_point3(eval::at(&spline.points, spline.closed, t));
        let up = world.transform_vector3(eval::up(&spline.points, spline.closed, t));
        if up.length_squared() > 1e-12 {
            gizmos.line(at, at + up.normalize() * tick, UP);
        }
    }
}

/// The two tangent handles of one knot, drawn where the curve actually leaves and arrives.
///
/// 🔴 Both come from `eval::tangents`, the same function the curve is evaluated with, so a handle
/// cannot draw a direction the curve does not take — and the three modes need no branch here,
/// because resolving a mode is that function's job and not this one's.
///
/// Both sides are drawn even for `Aligned`, which authors one: showing only the editable half hides
/// that the other moved with it.
fn handles(spline: &Spline, index: usize, knot: &Knot, world: &Mat4, gizmos: &mut Gizmos<'_>) {
    let colour = match knot.mode == TANGENT_AUTO {
        true => DERIVED,
        false => HANDLE,
    };
    let at = world.transform_point3(knot.position);
    let (count, last) = (spline.points.len(), spline.points.len() - 1);

    // An open curve's ends have one neighbour, so one of the two handles is not a direction the
    // curve takes anywhere.
    let leaving = (spline.closed || index != last)
        .then(|| eval::tangents(&spline.points, spline.closed, index, (index + 1) % count).0);
    // Negated because a handle points away from its knot while the evaluator's arriving tangent
    // points along the travel.
    let arriving = (spline.closed || index != 0).then(|| {
        -eval::tangents(
            &spline.points,
            spline.closed,
            (index + count - 1) % count,
            index,
        )
        .1
    });

    for handle in [leaving, arriving].into_iter().flatten() {
        let tip = at + world.transform_vector3(handle) * HANDLE_SCALE;
        gizmos.line(at, tip, colour);
        mark(tip, colour, gizmos);
    }
}

/// A Hermite tangent spans its whole segment, so drawing it full length buries the curve under its
/// own handles. A third is what every editor draws.
const HANDLE_SCALE: f32 = 1.0 / 3.0;

/// A small cross, which reads at any distance where a dot would vanish.
fn mark(at: Vec3, colour: Vec3, gizmos: &mut Gizmos<'_>) {
    const ARM: f32 = 0.06;
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        gizmos.line(at - axis * ARM, at + axis * ARM, colour);
    }
}

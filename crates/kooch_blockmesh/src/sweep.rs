//! [`SplineSweep`] — a profile carried along a [`Spline`](kooch_ecs::spline::Spline) (#1152).
//!
//! 🔴 The curve is read, never owned. A sweep sits on the entity that already has the `Spline`, so
//! the knots, their tangents and their radius are authored once and this is only what gets built
//! from them. The radius is the whole point: it is what tapers a trunk towards its tip and narrows
//! a pipe, and without it every sweep comes out the same thickness end to end.
//!
//! Produces a [`BlockMesh`] like every other shape, so face selection, extrude and colliders work
//! on the result with no code of their own.

use std::f32::consts::{FRAC_PI_4, SQRT_2, TAU};

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::reflect::{FieldChoice, FieldCondition, FieldRange};
use kooch_ecs::spline::{Knot, eval};

use crate::BlockMesh;
use crate::shapes::Builder;

/// A ring of `sides` corners.
pub const PROFILE_ROUND: u32 = 0;
/// Four corners, square to the curve's up — a beam or a kerb rather than a pipe.
pub const PROFILE_SQUARE: u32 = 1;

/// What the Inspector offers for `profile`. Scenes store the number, so these only append.
pub static PROFILE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Round",
        value: PROFILE_ROUND as i64,
    },
    FieldChoice {
        label: "Square",
        value: PROFILE_SQUARE as i64,
    },
];

/// `sides` means nothing to a square.
pub static SIDES_WHEN: FieldCondition = FieldCondition {
    field: "profile",
    values: &[PROFILE_ROUND as i64],
};

/// What to sweep along the curve on the same entity, and how finely.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Spline")]
pub struct SplineSweep {
    #[reflect(choices = PROFILE_CHOICES)]
    pub profile: u32,
    /// Corners around a round profile.
    #[reflect(shown_when = SIDES_WHEN)]
    pub sides: u32,
    /// Rings per curve segment. What decides whether a bend reads as a bend or as a crease.
    pub spans: u32,
    /// Closes an open sweep at both ends. A closed curve has no ends to cap, and neither has a
    /// walled one — its ends are always a rim, or the tunnel would have no mouth.
    pub caps: bool,
    /// How thick the wall is, zero meaning solid. Above zero the sweep is a tube you can walk
    /// through, and a ring thinner than this is where the passage ends.
    #[reflect(range = WALL_RANGE)]
    pub wall: f32,
}

/// A wall is a thickness, so it has no sense below zero; the ceiling is the widest radius a knot
/// can carry, past which every ring is solid anyway.
const WALL_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 10.0,
    step: 0.05,
};

impl Default for SplineSweep {
    fn default() -> Self {
        Self {
            profile: PROFILE_ROUND,
            sides: 8,
            spans: 8,
            caps: true,
            wall: 0.0,
        }
    }
}

impl Component for SplineSweep {}

/// Fewest and most corners a round profile is built with. Three is the fewest that encloses
/// anything; past the ceiling the corners are closer together than the millimetre the builder
/// welds at, and the ring would weld itself shut.
const MIN_SIDES: u32 = 3;
const MAX_SIDES: u32 = 64;

/// Ceiling on the rings of one sweep, so a long curve at a high `spans` cannot hang the editor
/// while the author is still typing the number.
const MAX_RINGS: usize = 4096;

/// Thinnest ring that still encloses anything. A knot below this contributes a point, which is
/// what puts a tip on a branch.
const MIN_RADIUS: f32 = 1.0e-3;

/// The profile swept along the curve, as an authoring mesh.
///
/// Empty where there is no curve to sweep: fewer than two knots is a point, and a mesh built from
/// it would be a shape with no inside.
pub fn sweep(points: &[Knot], closed: bool, of: &SplineSweep) -> BlockMesh {
    let segments = eval::segments(points, closed);
    if segments == 0 {
        return BlockMesh::default();
    }
    let spans = (of.spans.max(1) as usize * segments).min(MAX_RINGS);
    // An open curve needs the ring at its far end; a closed one would land that ring on its first.
    let count = match closed {
        true => spans,
        false => spans + 1,
    };

    // NaN fails every comparison, so ask whether there IS a wall rather than whether there is none.
    let hollow = of.wall > 0.0;
    let twist = closing_twist(points, closed);
    let shell = |inset: f32| -> Vec<Vec<Vec3>> {
        (0..count)
            .map(|index| {
                let along = index as f32 / spans as f32;
                ring(points, closed, of, along, twist * along, inset)
            })
            .collect()
    };
    let rings = shell(0.0);
    let bore: Vec<Vec<Vec3>> = match hollow {
        true => shell(of.wall),
        false => Vec::new(),
    };

    let mut out = Builder::default();
    for index in 0..spans {
        let ahead = (index + 1) % count;
        let axis = (centre(&rings[index]) + centre(&rings[ahead])) / 2.0;
        walls(&mut out, &rings[index], &rings[ahead], axis);
        if hollow {
            bore_walls(&mut out, &bore[index], &bore[ahead], axis);
        }
    }
    if !closed {
        // Wound away from the ring beside it, which is inside the tube: an end facing the wrong way
        // is a hole that only shows up as a black mouth.
        let (first, last) = (centre(&rings[1]), centre(&rings[count - 2]));
        match hollow {
            // A rim, not a lid, and built whatever `caps` says — a plugged tunnel is not a tunnel,
            // but an unsealed shell is a leak. Where the bore has pinched shut the rim is a disc.
            true => {
                walls(&mut out, &rings[0], &bore[0], first);
                walls(&mut out, &rings[count - 1], &bore[count - 1], last);
            }
            false if of.caps => {
                cap(&mut out, &rings[0], first);
                cap(&mut out, &rings[count - 1], last);
            }
            false => {}
        }
    }
    out.finish()
}

/// The quads between two rings, each wound away from `inside`.
fn walls(out: &mut Builder, here: &[Vec3], next: &[Vec3], inside: Vec3) {
    for corner in 0..here.len() {
        let ahead = (corner + 1) % here.len();
        // 🔴 A ring collapsed to a point welds its corners together, and a quad with two corners
        // on one position is a face with no area that `Adjacency::of` counts as a leak.
        let quad = dedup(&[here[corner], here[ahead], next[ahead], next[corner]]);
        if quad.len() > 2 {
            out.face_away(&quad, inside);
        }
    }
}

/// The same quads turned in, so the bore's normals face the passage rather than the rock.
fn bore_walls(out: &mut Builder, here: &[Vec3], next: &[Vec3], axis: Vec3) {
    for corner in 0..here.len() {
        let ahead = (corner + 1) % here.len();
        let quad = dedup(&[here[corner], here[ahead], next[ahead], next[corner]]);
        if quad.len() > 2 {
            out.face_toward(&quad, axis);
        }
    }
}

/// One end closed off, unless its ring has collapsed to a point and already closes itself.
fn cap(out: &mut Builder, ring: &[Vec3], inside: Vec3) {
    if dedup(ring).len() < 3 {
        return;
    }
    out.face_away(ring, inside);
}

/// The points of `ring` with repeats taken out, keeping their order.
fn dedup(ring: &[Vec3]) -> Vec<Vec3> {
    let mut kept: Vec<Vec3> = Vec::with_capacity(ring.len());
    for point in ring {
        if !kept.contains(point) {
            kept.push(*point);
        }
    }
    kept
}

fn centre(ring: &[Vec3]) -> Vec3 {
    ring.iter().copied().sum::<Vec3>() / ring.len() as f32
}

/// One ring of the profile, at `along` of the way down the curve.
///
/// 🔴 The frame comes from `eval::up`, which is rotation-minimising. The Frenet normal flips
/// through an inflection, and a swept ring that flips turns the mesh inside out over one span.
fn ring(
    points: &[Knot],
    closed: bool,
    of: &SplineSweep,
    along: f32,
    twist: f32,
    inset: f32,
) -> Vec<Vec3> {
    let at = eval::at(points, closed, along);
    // Clamped here and not against the whole curve: a trunk that tapers to a tip would otherwise
    // force the wall to nothing everywhere, and a tunnel stays hollow until its own ring pinches.
    let radius = (eval::radius_at(points, closed, along).max(0.0) - inset).max(0.0);
    let eval::Frame { up, side, .. } = eval::frame(points, closed, along);
    let (corners, offset, reach) = match of.profile {
        // A square's corners sit on the diagonal, so its radius has to carry out to them for its
        // sides to land at the radius the author asked for.
        PROFILE_SQUARE => (4, FRAC_PI_4, SQRT_2),
        _ => (of.sides.clamp(MIN_SIDES, MAX_SIDES), 0.0, 1.0),
    };
    if !(radius > MIN_RADIUS) {
        // A tip: every corner on the curve itself, which the builder welds into one point.
        return vec![at; corners as usize];
    }
    (0..corners)
        .map(|corner| {
            let angle = offset + twist + TAU * corner as f32 / corners as f32;
            at + (up * angle.cos() + side * angle.sin()) * radius * reach
        })
        .collect()
}

/// How far the profile has to be turned back, spread along the curve, for a closed sweep to meet
/// itself.
///
/// 🔴 Transport around a loop does not come back to the up it started with — that residue is the
/// curve's holonomy, and it is geometry rather than a bug. Left alone it lands entirely in the
/// last span, as one visible kink. Spread, it reads as a gentle twist nobody notices.
fn closing_twist(points: &[Knot], closed: bool) -> f32 {
    if !closed {
        return 0.0;
    }
    let tangent = eval::tangent(points, closed, 0.0);
    let (start, end) = (eval::up(points, closed, 0.0), eval::up(points, closed, 1.0));
    if !(tangent.length_squared() > 1.0e-12) {
        return 0.0;
    }
    // Signed about the tangent, so the correction turns back the way the drift went.
    let axis = tangent.normalize();
    -start.cross(end).dot(axis).atan2(start.dot(end))
}

#[cfg(test)]
mod tests;

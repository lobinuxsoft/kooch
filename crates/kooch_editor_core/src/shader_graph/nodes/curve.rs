//! The curve a `Curve` node carries, and the pieces the codegen writes it as (#1943).

use bevy_lookup_curve::{Knot, KnotInterpolation, LookupCurve, Tangent, TangentMode};
use serde::{Deserialize, Serialize};

/// Pieces a weighted cubic is cut into: WGSL has no root finder, so it is written as Hermites.
const WEIGHTED_CUTS: usize = 8;

/// A node's curve. Compared by shape: a knot's id is the editor's, and changes on every load.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct GraphCurve(pub(crate) LookupCurve);

/// One span of the curve between `x0` and `x1`, in a form WGSL evaluates directly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Piece {
    Flat {
        y: f32,
    },
    Line {
        y0: f32,
        y1: f32,
    },
    /// Cubic Hermite with the slopes already scaled to the span.
    Hermite {
        y0: f32,
        y1: f32,
        m0: f32,
        m1: f32,
    },
}

impl Default for GraphCurve {
    /// The identity over 0..1, so a node dropped in changes nothing until it is bent.
    fn default() -> Self {
        // Fields set one by one: `Knot::position` is `bevy_math`'s `Vec2`, not a type we name.
        let knot = |x: f32| {
            let mut knot = Knot {
                interpolation: KnotInterpolation::Cubic,
                left_tangent: Tangent {
                    slope: 1.0,
                    ..Tangent::default()
                },
                right_tangent: Tangent {
                    slope: 1.0,
                    ..Tangent::default()
                },
                ..Knot::default()
            };
            knot.position.x = x;
            knot.position.y = x;
            knot
        };
        Self(LookupCurve::new(vec![knot(0.0), knot(1.0)]))
    }
}

impl PartialEq for GraphCurve {
    fn eq(&self, other: &Self) -> bool {
        let same_tangent = |a: &Tangent, b: &Tangent| {
            a.slope == b.slope
                && a.weight == b.weight
                && matches!(
                    (a.mode, b.mode),
                    (TangentMode::Free, TangentMode::Free)
                        | (TangentMode::Aligned, TangentMode::Aligned)
                )
        };
        let (a, b) = (self.0.knots(), other.0.knots());
        a.len() == b.len()
            && a.iter().zip(b).all(|(a, b)| {
                a.position == b.position
                    && std::mem::discriminant(&a.interpolation)
                        == std::mem::discriminant(&b.interpolation)
                    && same_tangent(&a.left_tangent, &b.left_tangent)
                    && same_tangent(&a.right_tangent, &b.right_tangent)
            })
    }
}

impl GraphCurve {
    /// Where the curve starts and ends: below the first knot it holds the first value, past the
    /// last the last one, as `LookupCurve::lookup` does.
    pub(crate) fn ends(&self) -> Option<((f32, f32), (f32, f32))> {
        let knots = self.0.knots();
        let (first, last) = (knots.first()?, knots.last()?);
        Some((
            (first.position.x, first.position.y),
            (last.position.x, last.position.y),
        ))
    }

    /// Every span, as `(x0, x1, piece)`, in order. Spans of no width are dropped.
    pub(crate) fn pieces(&self) -> Vec<(f32, f32, Piece)> {
        let knots = self.0.knots();
        let mut out = Vec::new();
        for pair in knots.windows(2) {
            let [a, b] = pair else { continue };
            let (x0, x1) = (a.position.x, b.position.x);
            let dx = x1 - x0;
            if !(dx > 0.0) {
                continue;
            }
            let (y0, y1) = (a.position.y, b.position.y);
            match a.interpolation {
                KnotInterpolation::Constant => out.push((x0, x1, Piece::Flat { y: y0 })),
                KnotInterpolation::Linear => out.push((x0, x1, Piece::Line { y0, y1 })),
                KnotInterpolation::Cubic
                    if a.right_tangent.weight.is_none() && b.left_tangent.weight.is_none() =>
                {
                    let (m0, m1) = (a.right_tangent.slope * dx, b.left_tangent.slope * dx);
                    out.push((x0, x1, Piece::Hermite { y0, y1, m0, m1 }));
                }
                KnotInterpolation::Cubic => self.cut_weighted(x0, x1, &mut out),
            }
        }
        out
    }

    /// A weighted span as Hermites through the curve's own values, slopes by central difference.
    fn cut_weighted(&self, x0: f32, x1: f32, out: &mut Vec<(f32, f32, Piece)>) {
        let step = (x1 - x0) / WEIGHTED_CUTS as f32;
        let h = step * 0.01;
        let slope = |x: f32| (self.0.lookup(x + h) - self.0.lookup(x - h)) / (2.0 * h);
        for i in 0..WEIGHTED_CUTS {
            let a = x0 + step * i as f32;
            let b = if i + 1 == WEIGHTED_CUTS { x1 } else { a + step };
            // Inside the span, so the difference never reads across a knot.
            let (sa, sb) = (slope(a + h), slope(b - h));
            out.push((
                a,
                b,
                Piece::Hermite {
                    y0: self.0.lookup(a),
                    y1: self.0.lookup(b),
                    m0: sa * (b - a),
                    m1: sb * (b - a),
                },
            ));
        }
    }
}

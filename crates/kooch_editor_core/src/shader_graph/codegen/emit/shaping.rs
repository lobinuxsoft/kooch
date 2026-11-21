//! Smooth min/max, terraces and curves: the nodes that shape a planet's relief (#1943).

use egui_snarl::NodeId;

use super::Body;
use crate::shader_graph::{GraphCurve, Piece};

/// The smallest rounding or ramp a node divides by: a pin left at 0 must not make a NaN.
const TINY: &str = "0.00001";

impl Body<'_> {
    /// Quílez's polynomial smooth min, or max through `-min(-a, -b)`. `k` 0 is the plain one.
    pub(super) fn smooth(&mut self, id: NodeId, max: bool) -> Result<String, String> {
        // 🔴 Fallbacks match `pin_default`: a pin typed at its default is not stored.
        let (a, b) = (self.input(id, 0)?, self.input(id, 1)?);
        let k = self.input_or(id, 2, "vec4<f32>(1.0)")?;
        let k = self.local(&format!("vec4<f32>(max({k}.x, 0.0))"));
        let h = self.local(&format!("max({k} - abs({a} - {b}), vec4<f32>(0.0))"));
        let fillet = format!("{h} * {h} * 0.25 / max({k}, vec4<f32>({TINY}))");
        Ok(match max {
            false => format!("min({a}, {b}) - {fillet}"),
            true => format!("max({a}, {b}) + {fillet}"),
        })
    }

    /// Steps `step` high; each rises along a smoothstep over the last `softness` of its width, so
    /// a step joins the next with no kink.
    pub(super) fn terrace(&mut self, id: NodeId) -> Result<String, String> {
        let value = self.input(id, 0)?;
        let step = self.input_or(id, 1, "vec4<f32>(1.0)")?;
        let softness = self.input_or(id, 2, "vec4<f32>(0.5)")?;
        let step = self.local(&format!("vec4<f32>(max(abs({step}.x), {TINY}))"));
        let t = self.local(&format!("{value} / {step}"));
        let low = format!("vec4<f32>(min(1.0 - clamp({softness}.x, 0.0, 1.0), 1.0 - {TINY}))");
        Ok(format!(
            "(floor({t}) + smoothstep({low}, vec4<f32>(1.0), fract({t}))) * {step}"
        ))
    }

    /// Each span picked by `select` from the first knot on; outside the knots the ends hold.
    pub(super) fn curve(&mut self, id: NodeId, curve: &GraphCurve) -> Result<String, String> {
        let value = self.input(id, 0)?;
        let Some(((_, y_first), (x_last, y_last))) = curve.ends() else {
            return Ok("vec4<f32>(0.0)".to_owned());
        };
        let x = self.local(&value);
        let mut acc = self.local(&format!("vec4<f32>({y_first:?})"));
        for (x0, x1, piece) in curve.pieces() {
            let s = self.local(&format!(
                "clamp(({x} - vec4<f32>({x0:?})) / {:?}, vec4<f32>(0.0), vec4<f32>(1.0))",
                x1 - x0
            ));
            let span = match piece {
                Piece::Flat { y } => format!("vec4<f32>({y:?})"),
                Piece::Line { y0, y1 } => {
                    format!("mix(vec4<f32>({y0:?}), vec4<f32>({y1:?}), {s})")
                }
                Piece::Hermite { y0, y1, m0, m1 } => format!(
                    "(2.0 * {s} * {s} * {s} - 3.0 * {s} * {s} + 1.0) * {y0:?} \
                     + ({s} * {s} * {s} - 2.0 * {s} * {s} + {s}) * {m0:?} \
                     + ({s} * {s} * {s} - {s} * {s}) * {m1:?} \
                     + (3.0 * {s} * {s} - 2.0 * {s} * {s} * {s}) * {y1:?}"
                ),
            };
            acc = self.local(&format!("select({acc}, {span}, {x} >= vec4<f32>({x0:?}))"));
        }
        Ok(format!(
            "select({acc}, vec4<f32>({y_last:?}), {x} >= vec4<f32>({x_last:?}))"
        ))
    }
}

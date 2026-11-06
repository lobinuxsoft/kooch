//! Curves (#1827): a node's descendants lay out straight and bend when drawn, so nothing but the
//! draw and the pointer ever knows. The line is resampled by length here, once, for both.

use glam::Vec2;
use kooch_ecs::CURVE_POINTS;

use crate::{UiNode, UiTree};

/// Steps a curve is measured in; the line's points land between them.
const STEPS: usize = 256;

/// `CURVE_POINTS` points evenly spaced along a curve.
pub(crate) type UiLine = [Vec2; CURVE_POINTS];

fn at([start, control, end]: [Vec2; 3], t: f32) -> Vec2 {
    start.lerp(control, t).lerp(control.lerp(end, t), t)
}

/// Evenly spaced by length, not by `t`: a lopsided curve would stretch what is on it.
fn resampled(curve: [Vec2; 3]) -> UiLine {
    let mut lengths = [0.0; STEPS + 1];
    let mut last = curve[0];
    for step in 1..=STEPS {
        let point = at(curve, step as f32 / STEPS as f32);
        lengths[step] = lengths[step - 1] + last.distance(point);
        last = point;
    }
    let total = lengths[STEPS];
    let mut step = 0;
    std::array::from_fn(|index| {
        let want = total * index as f32 / (CURVE_POINTS - 1) as f32;
        while step + 1 < STEPS && lengths[step + 1] < want {
            step += 1;
        }
        let span = lengths[step + 1] - lengths[step];
        let part = if span > 0.0 {
            (want - lengths[step]) / span
        } else {
            0.0
        };
        at(curve, (step as f32 + part) / STEPS as f32)
    })
}

impl UiTree {
    /// Bends what is inside `node` along three points across it; `None` leaves it straight.
    pub fn set_curve(&mut self, node: UiNode, curve: Option<[[f32; 2]; 3]>) {
        if let Some(slot) = self.curve.get_mut(node.0 as usize) {
            *slot = curve.map(|points| points.map(Vec2::from));
        }
    }

    /// `node`'s curve in canvas units.
    pub(crate) fn line(&self, node: UiNode) -> Option<UiLine> {
        let index = node.0 as usize;
        let rect = self.rect.get(index)?;
        let curve = (*self.curve.get(index)?)?;
        Some(resampled(curve.map(|point| rect.min + point * rect.size)))
    }

    /// The nearest node above `node` with a curve.
    fn bent_by(&self, node: UiNode) -> Option<UiNode> {
        let mut next = self.parent(node);
        while let Some(above) = next {
            if self.curve.get(above.0 as usize)?.is_some() {
                return Some(above);
            }
            next = self.parent(above);
        }
        None
    }

    /// Where `point`, as seen on screen, sits in the layout of `curved`'s descendants.
    fn unbent(&self, curved: UiNode, point: Vec2) -> Option<Vec2> {
        let line = self.line(curved)?;
        let rect = self.rect.get(curved.0 as usize)?;
        let mut best = (f32::MAX, 0.0, 0.0);
        for (index, pair) in line.windows(2).enumerate() {
            let along = pair[1] - pair[0];
            let from = point - pair[0];
            let part = (from.dot(along) / along.length_squared().max(f32::EPSILON)).clamp(0.0, 1.0);
            let offset = from - along * part;
            let distance = offset.length_squared();
            if distance < best.0 {
                // The draw's normal is `along` turned toward +Y.
                let side = along.perp_dot(offset).signum();
                best = (distance, index as f32 + part, side * distance.sqrt());
            }
        }
        let across = best.1 / (CURVE_POINTS - 1) as f32;
        let middle = rect.min.y + rect.size.y * 0.5;
        Some(Vec2::new(
            rect.min.x + across * rect.size.x,
            middle + best.2,
        ))
    }

    /// Where `point`, as seen on screen, sits in `node`'s layout.
    pub fn straight(&self, node: UiNode, point: Vec2) -> Vec2 {
        self.bent_by(node)
            .and_then(|curved| self.unbent(curved, point))
            .unwrap_or(point)
    }

    /// [`straight`](Self::straight) for one point over many nodes: `last` keeps the curve's answer.
    pub(crate) fn straight_again(
        &self,
        node: UiNode,
        point: Vec2,
        last: &mut Option<(UiNode, Vec2)>,
    ) -> Vec2 {
        let Some(curved) = self.bent_by(node) else {
            return point;
        };
        if let Some((known, mapped)) = *last
            && known == curved
        {
            return mapped;
        }
        let mapped = self.unbent(curved, point).unwrap_or(point);
        *last = Some((curved, mapped));
        mapped
    }
}

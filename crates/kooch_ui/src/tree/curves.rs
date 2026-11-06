//! Curves (#1827, #1832): a node's descendants lay out straight and bend when drawn, so nothing but
//! the draw and the pointer ever knows. The path is `kooch_ecs`'s spline, flat, and is resampled by
//! length here, once, for both.

use glam::{Vec2, Vec3};
use kooch_ecs::CURVE_POINTS;
use kooch_ecs::spline::{Knot, arc, eval};

use crate::{UiNode, UiTree};

/// Most points a path takes: a typo must not become a table nothing can walk.
const MAX_POINTS: usize = 64;
/// Fewest points a closed path keeps: two would be a line walked twice.
const CLOSED_MIN: usize = 3;
/// Samples per stretch between two points, for what draws the path.
const SAMPLES: usize = 16;

/// `CURVE_POINTS` points evenly spaced along a curve.
pub(crate) type UiLine = [Vec2; CURVE_POINTS];

/// A curve through its points, as fractions of a node. Written `x y x y ...`, then `closed`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiPath {
    pub points: Vec<[f32; 2]>,
    pub closed: bool,
}

impl UiPath {
    /// `none` is no path.
    pub fn parse(text: &str) -> Result<Option<Self>, String> {
        let text = text.trim();
        if text == "none" {
            return Ok(None);
        }
        let (numbers, closed) = match text.strip_suffix("closed") {
            Some(numbers) => (numbers, true),
            None => (text, false),
        };
        let numbers: Result<Vec<f32>, _> = numbers.split_whitespace().map(str::parse).collect();
        let refused = || format!("`{text}` is not at least two points, or none");
        let numbers = numbers.map_err(|_| refused())?;
        if numbers.len() % 2 != 0 || numbers.iter().any(|number| !number.is_finite()) {
            return Err(refused());
        }
        let points: Vec<[f32; 2]> = numbers.chunks_exact(2).map(|n| [n[0], n[1]]).collect();
        let fewest = if closed { CLOSED_MIN } else { 2 };
        if !(fewest..=MAX_POINTS).contains(&points.len()) {
            return Err(refused());
        }
        Ok(Some(Self { points, closed }))
    }

    /// As the `.kcss` writes it.
    pub fn written(&self) -> String {
        let numbers = self.points.iter().flatten().map(|n| format!("{n:.3}"));
        let mut words: Vec<String> = numbers.collect();
        if self.closed {
            words.push("closed".to_owned());
        }
        words.join(" ")
    }

    /// Whether a point can go and leave a path.
    pub fn removable(&self) -> bool {
        self.points.len() > if self.closed { CLOSED_MIN } else { 2 }
    }

    /// Whether a point can join it.
    pub fn growable(&self) -> bool {
        self.points.len() < MAX_POINTS
    }

    /// Points along the path, each with the stretch it is on: what an editor draws and clicks.
    pub fn sampled(&self) -> Vec<(usize, [f32; 2])> {
        let knots = self.knots(Vec2::ZERO, Vec2::ONE);
        let steps = eval::segments(&knots, self.closed) * SAMPLES;
        (0..=steps)
            .map(|step| {
                let point = eval::at(&knots, self.closed, step as f32 / steps.max(1) as f32);
                (step / SAMPLES, [point.x, point.y])
            })
            .collect()
    }

    fn knots(&self, min: Vec2, size: Vec2) -> Vec<Knot> {
        let at = |point: &[f32; 2]| (min + Vec2::from(*point) * size).extend(0.0);
        self.points
            .iter()
            .map(|point| Knot::at(at(point)))
            .collect()
    }

    /// Evenly spaced by length, not by `t`: a lopsided curve would stretch what is on it.
    fn line(&self, min: Vec2, size: Vec2) -> UiLine {
        let knots = self.knots(min, size);
        let table = arc::walked(&knots, self.closed);
        let total = table.last().copied().unwrap_or(0.0);
        std::array::from_fn(|index| {
            let wanted = total * index as f32 / (CURVE_POINTS - 1) as f32;
            let point: Vec3 = eval::at(&knots, self.closed, arc::parameter_in(&table, wanted));
            point.truncate()
        })
    }
}

impl UiTree {
    /// Bends what is inside `node` along a path across it; `None` leaves it straight.
    pub fn set_curve(&mut self, node: UiNode, curve: Option<UiPath>) {
        if let Some(slot) = self.curve.get_mut(node.0 as usize) {
            *slot = curve;
        }
    }

    /// Whether `node`'s path joins its ends.
    pub(crate) fn curve_closed(&self, node: UiNode) -> bool {
        let curve = self.curve.get(node.0 as usize);
        curve.is_some_and(|curve| curve.as_ref().is_some_and(|path| path.closed))
    }

    /// `node`'s curve in canvas units.
    pub(crate) fn line(&self, node: UiNode) -> Option<UiLine> {
        let index = node.0 as usize;
        let rect = self.rect.get(index)?;
        let path = self.curve.get(index)?.as_ref()?;
        Some(path.line(rect.min, rect.size))
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

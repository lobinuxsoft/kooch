//! Curves (#1827, #1832): a node's descendants lay out straight and bend when drawn, so nothing but
//! the draw and the pointer ever knows. The path is `kooch_ecs`'s spline, flat, and is resampled by
//! length here, once, for both.

use std::f32::consts::{FRAC_PI_2, PI};

use glam::{Vec2, Vec3};
use kooch_ecs::CURVE_POINTS;
use kooch_ecs::spline::ops::{HANDLE_SCALE, steered};
use kooch_ecs::spline::{Knot, TANGENT_ALIGNED, TANGENT_BROKEN, arc, eval};

use crate::{UiNode, UiTree};

/// Most points a path takes: a typo must not become a table nothing can walk.
const MAX_POINTS: usize = 64;
/// Fewest points a closed path keeps: two would be a line walked twice.
const CLOSED_MIN: usize = 3;
/// Samples per stretch between two points, for what draws the path.
const SAMPLES: usize = 16;

/// `CURVE_POINTS` points evenly spaced along a curve.
pub(crate) type UiLine = [Vec2; CURVE_POINTS];

/// A curve through its points, as fractions of a node, tangents too. Written `x y`, `x y / lx ly`
/// when aligned or `x y | ax ay lx ly` when broken, point after point, then `closed`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiPath {
    pub points: Vec<Knot>,
    pub closed: bool,
}

/// A vector on the flat path.
fn flat(x: f32, y: f32) -> Vec3 {
    Vec3::new(x, y, 0.0)
}

/// The next two words as a vector.
fn vector<'a>(words: &mut impl Iterator<Item = &'a str>) -> Option<Vec3> {
    let mut number = || words.next()?.parse().ok().filter(|n: &f32| n.is_finite());
    Some(flat(number()?, number()?))
}

impl UiPath {
    /// `none` is no path.
    pub fn parse(text: &str) -> Result<Option<Self>, String> {
        let text = text.trim();
        if text == "none" {
            return Ok(None);
        }
        let (body, closed) = match text.strip_suffix("closed") {
            Some(body) => (body, true),
            None => (text, false),
        };
        let refused = || format!("`{text}` is not at least two points, or none");
        let mut words = body.split_whitespace().peekable();
        let mut points = Vec::new();
        while words.peek().is_some() {
            let mut knot = Knot::at(vector(&mut words).ok_or_else(refused)?);
            match words.peek().copied() {
                Some("/") => {
                    words.next();
                    knot.mode = TANGENT_ALIGNED;
                    knot.leaving = vector(&mut words).ok_or_else(refused)?;
                }
                Some("|") => {
                    words.next();
                    knot.mode = TANGENT_BROKEN;
                    knot.arriving = vector(&mut words).ok_or_else(refused)?;
                    knot.leaving = vector(&mut words).ok_or_else(refused)?;
                }
                _ => {}
            }
            points.push(knot);
        }
        let fewest = if closed { CLOSED_MIN } else { 2 };
        if !(fewest..=MAX_POINTS).contains(&points.len()) {
            return Err(refused());
        }
        Ok(Some(Self { points, closed }))
    }

    /// As the `.kcss` writes it.
    pub fn written(&self) -> String {
        let pair = |v: Vec3| format!("{:.4} {:.4}", v.x, v.y);
        let mut words: Vec<String> = self
            .points
            .iter()
            .map(|knot| match knot.mode {
                TANGENT_ALIGNED => format!("{} / {}", pair(knot.position), pair(knot.leaving)),
                TANGENT_BROKEN => format!(
                    "{} | {} {}",
                    pair(knot.position),
                    pair(knot.arriving),
                    pair(knot.leaving)
                ),
                _ => pair(knot.position),
            })
            .collect();
        if self.closed {
            words.push("closed".to_owned());
        }
        words.join(" ")
    }

    /// Two points, a straight run.
    pub fn segment() -> Self {
        let points = [flat(0.1, 0.5), flat(0.9, 0.5)].map(Knot::at).to_vec();
        Self {
            points,
            closed: false,
        }
    }

    /// Half a circle over the node, left to right.
    pub fn arc() -> Self {
        Self {
            points: round(-PI, 3),
            closed: false,
        }
    }

    /// Four points clockwise from the top, a fill's natural start. A cubic per quarter is 0.03 % of
    /// the radius off a true circle: under a tenth of a pixel on a UI.
    pub fn circle() -> Self {
        Self {
            points: round(-FRAC_PI_2, 4),
            closed: true,
        }
    }

    /// Four corners: broken tangents along each side keep them straight.
    pub fn square() -> Self {
        let corners = [
            flat(0.1, 0.1),
            flat(0.9, 0.1),
            flat(0.9, 0.9),
            flat(0.1, 0.9),
        ];
        let points = (0..4).map(|index| {
            let here = corners[index];
            Knot {
                mode: TANGENT_BROKEN,
                arriving: corners[(index + 3) % 4] - here,
                leaving: corners[(index + 1) % 4] - here,
                ..Knot::at(here)
            }
        });
        Self {
            points: points.collect(),
            closed: true,
        }
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

    /// The tangents a point draws with, arriving then leaving, each pointing away from it. Read
    /// through the evaluator, so a handle sits on the curve drawn; an open path's ends lack one.
    fn tangents(&self, index: usize) -> [Option<Vec3>; 2] {
        let (count, closed) = (self.points.len(), self.closed);
        let knots = self.points.as_slice();
        if index >= count || count < 2 {
            return [None, None];
        }
        let arriving = (closed || index > 0)
            .then(|| -eval::tangents(knots, closed, (index + count - 1) % count, index).1);
        let leaving = (closed || index + 1 < count)
            .then(|| eval::tangents(knots, closed, index, (index + 1) % count).0);
        [arriving, leaving]
    }

    /// Where a point's handles sit, arriving then leaving.
    pub fn grips(&self, index: usize) -> [Option<[f32; 2]>; 2] {
        let Some(knot) = self.points.get(index) else {
            return [None, None];
        };
        self.tangents(index).map(|tangent| {
            let grip = knot.position + tangent? * HANDLE_SCALE;
            Some([grip.x, grip.y])
        })
    }

    /// Drags a point's handle to `[x, y]`.
    pub fn steer(&mut self, index: usize, arriving: bool, [x, y]: [f32; 2]) {
        if let Some(knot) = self.points.get_mut(index) {
            let vector = (flat(x, y) - knot.position) / HANDLE_SCALE;
            *knot = steered(*knot, arriving, vector);
        }
    }

    /// A point's mode, keeping the curve it had: a tangent left at zero would follow its sibling
    /// until dragged.
    pub fn set_mode(&mut self, index: usize, mode: u32) {
        let [arriving, leaving] = self.tangents(index);
        if let Some(knot) = self.points.get_mut(index) {
            knot.mode = mode;
            knot.arriving = arriving.or(leaving.map(|v| -v)).unwrap_or_default();
            knot.leaving = leaving.or(arriving.map(|v| -v)).unwrap_or_default();
        }
    }

    fn knots(&self, min: Vec2, size: Vec2) -> Vec<Knot> {
        let scale = size.extend(1.0);
        let scaled = |knot: &Knot| Knot {
            position: min.extend(0.0) + knot.position * scale,
            arriving: knot.arriving * scale,
            leaving: knot.leaving * scale,
            ..*knot
        };
        self.points.iter().map(scaled).collect()
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

/// `count` points a quarter turn apart from `start`, on a circle filling most of the node, with
/// the tangents of a cubic: Catmull-Rom alone bulges between so few.
fn round(start: f32, count: usize) -> Vec<Knot> {
    const RADIUS: f32 = 0.4;
    // A Bezier handle of 4/3 tan(turn / 4), as a Hermite tangent: three times that.
    let tangent = RADIUS * 4.0 * (FRAC_PI_2 / 4.0).tan();
    (0..count)
        .map(|index| {
            let (sin, cos) = (start + FRAC_PI_2 * index as f32).sin_cos();
            Knot {
                mode: TANGENT_ALIGNED,
                leaving: flat(-sin, cos) * tangent,
                ..Knot::at(flat(0.5 + cos * RADIUS, 0.5 + sin * RADIUS))
            }
        })
        .collect()
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
    pub(super) fn bent_by(&self, node: UiNode) -> Option<UiNode> {
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
    pub(super) fn unbent(&self, curved: UiNode, point: Vec2) -> Option<Vec2> {
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
        if self.flipped(node) {
            return self.unflipped(node, point);
        }
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
        if self.flipped(node) {
            return self.unflipped(node, point);
        }
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

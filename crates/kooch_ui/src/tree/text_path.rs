//! Text set along a curve (#1880): each glyph is placed and turned on the line the shader bends
//! quads along, so Vello draws it from its outline instead of a label warped strip by strip.

use glam::Vec2;
use kooch_ecs::CURVE_POINTS;

use super::curves::UiLine;
use super::scroll::UNCLIPPED;
use crate::{UiNode, UiTree};

/// A node's curve in physical pixels, as the draw hands it to the shader.
pub(super) struct TextPath {
    points: UiLine,
    left: f32,
    width: f32,
    middle: f32,
    closed: bool,
}

impl UiTree {
    /// The curve `node`'s text is set along. `None` when a flip or transform inside the curve
    /// moves it, or a clip or mask cuts it: those are tested in the layout, so the quad bends.
    pub(super) fn text_path(&self, node: UiNode, scale: f32) -> Option<TextPath> {
        let curved = self.bent_by(node)?;
        let moved = self
            .upwards(node)
            .take_while(|&at| at != curved)
            .any(|at| self.flip_of(at) != 0 || self.turn_of(at).is_some());
        let cut = self.upwards(node).any(|at| {
            self.mask(at).is_some() || self.fill_from(at) != 0 || self.rounded(at).is_some()
        });
        if moved || cut || self.clip(node)? != UNCLIPPED {
            return None;
        }
        let rect = self.rect(curved)?;
        Some(TextPath {
            points: self.line(curved)?.map(|point| point * scale),
            left: rect.min.x * scale,
            width: rect.size.x * scale,
            middle: (rect.min.y + rect.size.y * 0.5) * scale,
            closed: self.curve_closed(curved),
        })
    }
}

impl TextPath {
    /// Where the layout's `pixel` lands and the line's direction there: `ui_bent` in `ui.wgsl`.
    pub(super) fn at(&self, pixel: Vec2) -> (Vec2, Vec2) {
        let last = CURVE_POINTS - 1;
        let along = (pixel.x - self.left) / self.width.max(1.0) * last as f32;
        let index = along.clamp(0.0, (last - 1) as f32) as usize;
        let part = along - index as f32;
        let point = |at: usize| self.points.get(at).copied().unwrap_or_default();
        let (here, next) = (point(index), point(index + 1));
        let behind = match self.closed && index == 0 {
            true => last - 1,
            false => index.max(1) - 1,
        };
        let ahead = match self.closed && index + 2 > last {
            true => 1,
            false => (index + 2).min(last),
        };
        let before = next - point(behind);
        let after = point(ahead) - here;
        let tangent = before.lerp(after, part.clamp(0.0, 1.0));
        let unit = tangent / tangent.length().max(0.00001);
        let away = pixel.y - self.middle;
        (here.lerp(next, part) + unit.perp() * away, unit)
    }
}

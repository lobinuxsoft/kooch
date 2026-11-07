//! Shape masks (#1790): what a node cuts by more than its rectangle. The GPU does it per pixel in
//! `ui.wgsl`; this is the same test for the pointer, which must not hit what cannot be seen.

use glam::Vec2;
use kooch_core::Guid;
use taffy::Overflow;

use crate::{GlyphAtlas, UiNode, UiTree};

/// A mask's alpha from which the pointer counts it as solid.
const SOLID: u8 = 128;

/// The image a node cuts by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiMaskImage {
    pub guid: Guid,
    /// Black cuts as transparent does, for images with no alpha to read.
    pub luminance: bool,
}

/// What cuts a node besides its box.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct UiCut {
    pub(crate) image: Option<UiMaskImage>,
    fill_from: u32,
}

/// A rounded box in canvas units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct UiShape {
    pub(crate) min: Vec2,
    pub(crate) size: Vec2,
    pub(crate) radius: f32,
}

impl UiShape {
    fn holds(&self, point: Vec2) -> bool {
        let half = self.size * 0.5;
        let radius = self.radius.min(half.x).min(half.y);
        let q = (point - self.min - half).abs() - half + radius;
        q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0) - radius <= 0.0
    }
}

impl UiTree {
    pub fn set_mask(&mut self, node: UiNode, mask: Option<UiMaskImage>) {
        if let Some(slot) = self.mask.get_mut(node.0 as usize) {
            slot.image = mask;
        }
    }

    /// Where `node` starts showing, as an index in `-kooch-fill-from`'s words; 0 shows it whole.
    pub fn set_fill_from(&mut self, node: UiNode, from: u32) {
        if let Some(slot) = self.mask.get_mut(node.0 as usize) {
            slot.fill_from = from;
        }
    }

    pub fn fill_from(&self, node: UiNode) -> u32 {
        self.mask
            .get(node.0 as usize)
            .map_or(0, |cut| cut.fill_from)
    }

    /// The image whose alpha cuts `node` and what is inside it.
    pub fn mask(&self, node: UiNode) -> Option<UiMaskImage> {
        self.mask.get(node.0 as usize)?.image
    }

    /// The rounded padding box `node` cuts its children to. A square one is the rectangular clip's
    /// job, which costs no mask.
    pub(crate) fn rounded(&self, node: UiNode) -> Option<UiShape> {
        let index = node.0 as usize;
        let style = self.layout.style(*self.taffy.get(index)?).ok()?;
        let visible = Overflow::Visible;
        if style.overflow.x == visible && style.overflow.y == visible {
            return None;
        }
        let (rect, border) = (self.rect.get(index)?, *self.border.get(index)?);
        let radius = self.paint.get(index)?.radius - border;
        (radius > 0.0).then(|| UiShape {
            min: rect.min + border,
            size: rect.size - border * 2.0,
            radius,
        })
    }

    /// Whether `node`'s image mask, if any, is solid at `point`. Without an atlas, or before the
    /// image reached it, the node counts as unmasked, as it draws.
    fn solid(&self, node: UiNode, point: Vec2, atlas: Option<&GlyphAtlas>) -> bool {
        let found = self.mask(node).zip(atlas).and_then(|(mask, atlas)| {
            // Any raster of it answers whether a point is solid.
            let [x0, y0, x1, y1] = atlas.image(mask.guid, [0, 0])?.rect;
            let rect = self.rect.get(node.0 as usize)?;
            let at = ((point - rect.min) / rect.size).clamp(Vec2::ZERO, Vec2::ONE);
            let x = x0 + (at.x * (x1 - x0).saturating_sub(1) as f32) as u32;
            let y = y0 + (at.y * (y1 - y0).saturating_sub(1) as f32) as u32;
            let texel = (y as usize * atlas.side() as usize + x as usize) * 4;
            let [r, g, b, a] = *atlas.pixels().get(texel..texel + 4)? else {
                return None;
            };
            Some(match mask.luminance {
                true => {
                    let bright = 0.2125 * r as f32 + 0.7154 * g as f32 + 0.0721 * b as f32;
                    (bright * a as f32 / 255.0) as u8
                }
                false => a,
            })
        });
        found.is_none_or(|alpha| alpha >= SOLID)
    }

    /// Whether `point` survives every mask over `node`: its own image and all its ancestors'.
    pub(super) fn unmasked(&self, node: UiNode, point: Vec2, atlas: Option<&GlyphAtlas>) -> bool {
        if !self.solid(node, point, atlas) {
            return false;
        }
        let mut next = self.parent(node);
        while let Some(above) = next {
            let cut = self.rounded(above).is_some_and(|shape| !shape.holds(point));
            if cut || !self.solid(above, point, atlas) {
                return false;
            }
            next = self.parent(above);
        }
        true
    }
}

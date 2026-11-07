//! Flips (#1840): a node and what is inside it lay out as usual and mirror about its centre when
//! drawn, so only the draw and the pointer know. Undoing the draw here takes transforms along.

use glam::Vec2;

use crate::{UiNode, UiRect, UiTree};

/// Mirrors left to right.
pub const FLIP_X: u32 = 1;
/// Mirrors top to bottom.
pub const FLIP_Y: u32 = 2;

impl UiTree {
    /// Mirrors `node` and what is inside it: [`FLIP_X`], [`FLIP_Y`] or both.
    pub fn set_flip(&mut self, node: UiNode, flip: u32) {
        if let Some(slot) = self.flip.get_mut(node.0 as usize) {
            *slot = flip;
        }
    }

    pub fn flip_of(&self, node: UiNode) -> u32 {
        self.flip.get(node.0 as usize).copied().unwrap_or(0)
    }

    /// `node` and the nodes above it, nearest first.
    fn upwards(&self, node: UiNode) -> impl Iterator<Item = UiNode> + '_ {
        std::iter::successors(Some(node), |&at| self.parent(at))
    }

    /// Whether a flip or a transform moves `node`.
    pub(super) fn flipped(&self, node: UiNode) -> bool {
        self.upwards(node)
            .any(|at| self.flip_of(at) != 0 || self.turn_of(at).is_some())
    }

    /// `point` across `node`'s flip, which is its own inverse.
    fn across(&self, node: UiNode, point: Vec2) -> Vec2 {
        let flip = self.flip_of(node);
        let Some(rect) = self.rect(node).filter(|_| flip != 0) else {
            return point;
        };
        let far = rect.min * 2.0 + rect.size;
        Vec2::new(
            if flip & FLIP_X != 0 {
                far.x - point.x
            } else {
                point.x
            },
            if flip & FLIP_Y != 0 {
                far.y - point.y
            } else {
                point.y
            },
        )
    }

    /// Where `point`, as seen on screen, sits in the layout of a `node` something mirrors: the draw
    /// undone from the root down, its nearest curve included.
    pub(super) fn unflipped(&self, node: UiNode, point: Vec2) -> Vec2 {
        let curved = self.bent_by(node);
        let chain: Vec<UiNode> = self.upwards(node).collect();
        chain.iter().rev().fold(point, |point, &at| {
            let point = self.across(at, self.unturned(at, point));
            match curved == Some(at) {
                true => self.unbent(at, point).unwrap_or(point),
                false => point,
            }
        })
    }

    /// The box around `node`'s rect where the flips and transforms draw it, curves aside.
    pub fn shown(&self, node: UiNode) -> Option<UiRect> {
        let rect = self.rect(node)?;
        let turned = |point| {
            self.upwards(node).fold(point, |point, at| {
                let point = self.across(at, point);
                self.turn_of(at)
                    .map_or(point, |turn| super::transforms::projected(turn, point))
            })
        };
        let far = rect.min + rect.size;
        let corners = [
            turned(rect.min),
            turned(Vec2::new(far.x, rect.min.y)),
            turned(Vec2::new(rect.min.x, far.y)),
            turned(far),
        ];
        let min = corners.iter().copied().fold(corners[0], Vec2::min);
        let max = corners.iter().copied().fold(corners[0], Vec2::max);
        Some(UiRect {
            min,
            size: max - min,
        })
    }

    /// The axes `node` ends up mirrored on, flips above it included.
    pub fn flip_total(&self, node: UiNode) -> u32 {
        self.upwards(node)
            .fold(0, |total, at| total ^ self.flip_of(at))
    }
}

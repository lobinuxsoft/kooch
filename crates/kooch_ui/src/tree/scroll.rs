//! Scrolling and clipping (#1775): an `overflow` node cuts what its children show, and a `scroll`
//! one moves them by an offset clamped to its content.

use glam::Vec2;
use taffy::Overflow;

use super::{UiNode, UiTree};

pub(super) const UNCLIPPED: [f32; 4] = [f32::MIN, f32::MIN, f32::MAX, f32::MAX];

impl UiTree {
    /// What a child of `index` can be seen in: its own clip, cut to its padding box on each axis
    /// whose `overflow` is not `visible`.
    pub(super) fn cut(&self, index: usize) -> [f32; 4] {
        let mut clip = self.clip[index];
        let Ok(style) = self.layout.style(self.taffy[index]) else {
            return clip;
        };
        let rect = self.rect[index];
        let border = self.border[index];
        let (min, max) = (rect.min + border, rect.min + rect.size - border);
        if style.overflow.x != Overflow::Visible {
            clip[0] = clip[0].max(min.x);
            clip[2] = clip[2].min(max.x);
        }
        if style.overflow.y != Overflow::Visible {
            clip[1] = clip[1].max(min.y);
            clip[3] = clip[3].min(max.y);
        }
        clip
    }

    /// Wholly outside what its ancestors show: not drawn, rasterized or picked.
    pub fn clipped(&self, node: UiNode) -> bool {
        let i = node.0 as usize;
        let (Some(rect), Some(clip)) = (self.rect.get(i), self.clip.get(i)) else {
            return true;
        };
        let max = rect.min + rect.size;
        !(rect.min.x < clip[2] && rect.min.y < clip[3] && max.x > clip[0] && max.y > clip[1])
    }

    /// `[x0, y0, x1, y1]` in canvas units of what can be seen of `node`.
    pub fn clip(&self, node: UiNode) -> Option<[f32; 4]> {
        self.clip.get(node.0 as usize).copied()
    }

    /// Scrolls the innermost `overflow: scroll` node under `point` by `delta` canvas units, clamped
    /// to its content. Returns whether anything moved; the next [`layout`](Self::layout) places it.
    pub fn scroll_at(&mut self, point: Vec2, delta: Vec2) -> bool {
        let Some(node) = (0..self.rect.len()).rev().find(|&i| {
            let rect = self.rect[i];
            let clip = self.clip[i];
            let inside = point.cmpge(rect.min).all() && point.cmplt(rect.min + rect.size).all();
            let seen =
                point.x >= clip[0] && point.y >= clip[1] && point.x < clip[2] && point.y < clip[3];
            inside && seen && !self.culled[i] && self.scrolls(i)
        }) else {
            return false;
        };
        let Ok(own) = self.layout.layout(self.taffy[node]) else {
            return false;
        };
        let limit = Vec2::new(own.scroll_width(), own.scroll_height());
        let next = (self.scroll[node] + delta).clamp(Vec2::ZERO, limit);
        if next == self.scroll[node] {
            return false;
        }
        self.scroll[node] = next;
        self.moved = true;
        true
    }

    /// The offset scrolling gave `node`'s children.
    pub fn scroll(&self, node: UiNode) -> Vec2 {
        self.scroll
            .get(node.0 as usize)
            .copied()
            .unwrap_or_default()
    }

    /// How far a `scroll` node can move on each axis; `None` for any other node.
    pub fn scroll_extent(&self, node: UiNode) -> Option<Vec2> {
        let index = node.0 as usize;
        if index >= self.taffy.len() || !self.scrolls(index) {
            return None;
        }
        let own = self.layout.layout(self.taffy[index]).ok()?;
        Some(Vec2::new(own.scroll_width(), own.scroll_height()))
    }

    pub(super) fn scrolls(&self, index: usize) -> bool {
        self.layout.style(self.taffy[index]).is_ok_and(|style| {
            style.overflow.x == Overflow::Scroll || style.overflow.y == Overflow::Scroll
        })
    }

    /// Scrolls each `scroll` ancestor of `node` just enough to show it whole, or its start when it
    /// is the larger. Reads this layout's rects: the move lands on the next one.
    pub fn reveal(&mut self, node: UiNode) -> bool {
        let Some(&rect) = self.rect.get(node.0 as usize) else {
            return false;
        };
        let mut moved = false;
        let mut current = self.parent(node);
        while let Some(parent) = current {
            let index = parent.0 as usize;
            if self.scrolls(index)
                && let Ok(own) = self.layout.layout(self.taffy[index])
            {
                let border = self.border[index];
                let view_min = self.rect[index].min + border;
                let view_max = self.rect[index].min + self.rect[index].size - border;
                let mut offset = self.scroll[index];
                for axis in 0..2 {
                    let (min, max) = (rect.min[axis], rect.min[axis] + rect.size[axis]);
                    if min < view_min[axis] {
                        offset[axis] -= view_min[axis] - min;
                    } else if max > view_max[axis] {
                        offset[axis] += (max - view_max[axis]).min(min - view_min[axis]);
                    }
                }
                let limit = Vec2::new(own.scroll_width(), own.scroll_height());
                let offset = offset.clamp(Vec2::ZERO, limit);
                if offset != self.scroll[index] {
                    self.scroll[index] = offset;
                    moved = true;
                }
            }
            current = self.parent(parent);
        }
        self.moved |= moved;
        moved
    }
}

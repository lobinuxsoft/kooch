//! Text sized by its parent: known only once the parent is laid out.

use super::{UiNode, UiTree};
use crate::layout::CANVAS_HEIGHT;

impl UiTree {
    /// `node`'s text as a share of its parent's height; `None` keeps the size its style gave it.
    pub fn set_font_share(&mut self, node: UiNode, share: Option<f32>) {
        if let Some(slot) = self.font_share.get_mut(node.0 as usize)
            && *slot != share
        {
            *slot = share;
            self.laid_out = None;
        }
    }

    /// Sizes shared text from the layout just run; whether any changed. The clamp stops a parent
    /// sized by its own text from growing or vanishing frame after frame.
    pub(super) fn fit_text(&mut self) -> bool {
        let mut changed = false;
        for index in 0..self.font_share.len() {
            let (Some(share), Some(parent)) = (
                self.font_share[index],
                self.rect.get(self.parent[index] as usize),
            ) else {
                continue;
            };
            let size = (parent.size.y * share).clamp(1.0, CANVAS_HEIGHT);
            let held = self.text[index].as_ref().map(|text| text.size);
            if held.is_some_and(|held| (held - size).abs() > 0.01) {
                self.set_text_size(UiNode(index as u32), size);
                changed = true;
            }
        }
        changed
    }
}

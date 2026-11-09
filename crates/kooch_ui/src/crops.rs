//! Vectors past the largest size class (#1880): a node bigger than [`class_side`]`(TOP_CLASS)` on
//! screen is drawn only where the view shows it, at its own pixels, instead of a capped drawing
//! magnified. Needs a view ([`UiTree::set_view`]) and no transform: the crop is in layout pixels.

use crate::atlas::{TOP_CLASS, class_side};
use crate::{UiNode, UiTree};

/// The part of a node drawn: its whole size, and the texels of it the view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Crop {
    pub full: [u32; 2],
    pub from: [u32; 2],
    /// Zero when nothing of the node shows.
    pub size: [u32; 2],
}

/// The largest crop side: the vector atlas's own limit, less its margin.
const LARGEST: f32 = 4094.0;

impl UiTree {
    /// What of the layout is on screen, in physical pixels; `None` draws every vector whole.
    pub fn set_view(&mut self, view: Option<[f32; 4]>) {
        self.view = view;
    }
}

/// The node's box in whole physical pixels, `[x0, y0, x1, y1]`, as the draw rounds it.
pub(crate) fn pixels_of(tree: &UiTree, node: UiNode, scale: f32) -> Option<[f32; 4]> {
    let rect = tree.rect(node)?;
    let min = (rect.min * scale).round();
    let max = ((rect.min + rect.size) * scale).round();
    Some([min.x, min.y, max.x, max.y])
}

/// The crop `node` is drawn with, or `None` when it fits a size class.
pub(crate) fn crop_of(tree: &UiTree, node: UiNode, scale: f32) -> Option<Crop> {
    let view = tree.view?;
    let [x0, y0, x1, y1] = pixels_of(tree, node, scale)?;
    crop_in([x0, y0, x1, y1], view, tree.zoom_of(node))
}

/// The crop of a box of physical pixels, `None` when it fits a size class or is transformed.
pub(crate) fn crop_in([x0, y0, x1, y1]: [f32; 4], view: [f32; 4], zoom: f32) -> Option<Crop> {
    if !cfg!(feature = "vello") || (zoom - 1.0).abs() > 1e-3 {
        return None;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    if !(w.max(h) > class_side(TOP_CLASS) as f32) {
        return None;
    }
    let from = [
        (view[0] - x0).max(0.0).floor(),
        (view[1] - y0).max(0.0).floor(),
    ];
    let to = [(view[2] - x0).min(w).ceil(), (view[3] - y0).min(h).ceil()];
    let size = [
        (to[0] - from[0]).clamp(0.0, LARGEST),
        (to[1] - from[1]).clamp(0.0, LARGEST),
    ];
    Some(Crop {
        full: [w.ceil() as u32, h.ceil() as u32],
        from: from.map(|v| v as u32),
        size: size.map(|v| v as u32),
    })
}

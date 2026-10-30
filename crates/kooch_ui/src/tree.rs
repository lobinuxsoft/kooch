//! A document's nodes: taffy owns structure and layout, the parallel arrays own the rest. A
//! [`UiNode`] indexes all of them, so nothing outside needs taffy's slotmap keys.

use glam::Vec2;
use taffy::{AvailableSpace, NodeId, Size, Style, TaffyTree};

/// A node of one [`UiTree`]: dense from the root, which is `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UiNode(pub u32);

/// Where a node landed, in the document's logical canvas: top-left origin, y down.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UiRect {
    pub min: Vec2,
    pub size: Vec2,
}

/// One document's node tree. Grows only: a document that changes shape is rebuilt whole, and
/// hiding a node is `Display::None`, not removal.
#[derive(Debug)]
pub struct UiTree {
    layout: TaffyTree<()>,
    taffy: Vec<NodeId>,
    /// `u32::MAX` for the root. Always a lower index than the child, which is what lets one forward
    /// pass turn taffy's parent-relative positions into absolute ones.
    parent: Vec<u32>,
    name: Vec<Option<Box<str>>>,
    material: Vec<u32>,
    rect: Vec<UiRect>,
    /// Size the rects were computed for; `None` once anything changed.
    laid_out: Option<Vec2>,
}

// SAFETY: taffy's lengths are `!Send` only for the pointer a `calc()` length carries, and taffy is
// built without `calc`, so no length here holds one. Bevy's `UiTree` makes the same claim.
unsafe impl Send for UiTree {}
unsafe impl Sync for UiTree {}

impl UiTree {
    pub const ROOT: UiNode = UiNode(0);
    /// The default UI material, until a style names one.
    pub const NO_MATERIAL: u32 = u32::MAX;

    pub fn new(root: Style) -> Self {
        let mut tree = Self {
            layout: TaffyTree::new(),
            taffy: Vec::new(),
            parent: Vec::new(),
            name: Vec::new(),
            material: Vec::new(),
            rect: Vec::new(),
            laid_out: None,
        };
        // Logical units: rounding here would snap to a pixel the screen does not have. The draw
        // rounds, in physical pixels.
        tree.layout.disable_rounding();
        if let Ok(root) = tree.layout.new_leaf(root) {
            tree.append(root, u32::MAX);
        }
        tree
    }

    /// Adds a node as the last child of `parent`. `None` when `parent` is not of this tree.
    pub fn push(&mut self, parent: UiNode, style: Style) -> Option<UiNode> {
        let above = *self.taffy.get(parent.0 as usize)?;
        let node = self.layout.new_leaf(style).ok()?;
        self.layout.add_child(above, node).ok()?;
        self.laid_out = None;
        Some(self.append(node, parent.0))
    }

    pub fn set_style(&mut self, node: UiNode, style: Style) {
        let Some(&id) = self.taffy.get(node.0 as usize) else {
            return;
        };
        if self.layout.set_style(id, style).is_ok() {
            self.laid_out = None;
        }
    }

    /// Lays the tree out in `available` logical units, unless it already is for that size and
    /// nothing changed since. Returns whether it ran.
    pub fn layout(&mut self, available: Size<AvailableSpace>) -> bool {
        let key = Vec2::new(space_key(available.width), space_key(available.height));
        if self.laid_out == Some(key) || self.taffy.is_empty() {
            return false;
        }
        if self
            .layout
            .compute_layout(self.taffy[0], available)
            .is_err()
        {
            return false;
        }
        for index in 0..self.taffy.len() {
            let Ok(own) = self.layout.layout(self.taffy[index]) else {
                continue;
            };
            let origin = match self.parent[index] {
                u32::MAX => Vec2::ZERO,
                parent => self.rect[parent as usize].min,
            };
            self.rect[index] = UiRect {
                min: origin + Vec2::new(own.location.x, own.location.y),
                size: Vec2::new(own.size.width, own.size.height),
            };
        }
        self.laid_out = Some(key);
        true
    }

    pub fn rect(&self, node: UiNode) -> Option<UiRect> {
        self.rect.get(node.0 as usize).copied()
    }

    /// The first node with this name, in creation order. A scan: the game asks once and keeps the
    /// handle.
    pub fn query(&self, name: &str) -> Option<UiNode> {
        self.name
            .iter()
            .position(|own| own.as_deref() == Some(name))
            .map(|index| UiNode(index as u32))
    }

    pub fn set_name(&mut self, node: UiNode, name: &str) {
        if let Some(own) = self.name.get_mut(node.0 as usize) {
            *own = Some(name.into());
        }
    }

    pub fn set_material(&mut self, node: UiNode, material: u32) {
        if let Some(own) = self.material.get_mut(node.0 as usize) {
            *own = material;
        }
    }

    pub fn material(&self, node: UiNode) -> Option<u32> {
        self.material.get(node.0 as usize).copied()
    }

    pub fn len(&self) -> usize {
        self.taffy.len()
    }

    pub fn is_empty(&self) -> bool {
        self.taffy.is_empty()
    }

    fn append(&mut self, node: NodeId, parent: u32) -> UiNode {
        let index = UiNode(self.taffy.len() as u32);
        self.taffy.push(node);
        self.parent.push(parent);
        self.name.push(None);
        self.material.push(Self::NO_MATERIAL);
        self.rect.push(UiRect::default());
        index
    }
}

/// One axis of the available space as a cache key: content-sized axes get sentinels no size takes.
fn space_key(space: AvailableSpace) -> f32 {
    match space {
        AvailableSpace::Definite(size) => size,
        AvailableSpace::MinContent => -1.0,
        AvailableSpace::MaxContent => -2.0,
    }
}

#[cfg(test)]
mod tests;

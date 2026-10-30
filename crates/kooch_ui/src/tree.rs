//! A document's nodes: taffy owns structure and layout, the parallel arrays own the rest. A
//! [`UiNode`] indexes all of them, so nothing outside needs taffy's slotmap keys.

use taffy::{NodeId, Style, TaffyTree};

/// A node of one [`UiTree`]: dense from the root, which is `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UiNode(pub u32);

/// One document's node tree. Grows only: a document that changes shape is rebuilt whole, and
/// hiding a node is `Display::None`, not removal.
#[derive(Debug)]
pub struct UiTree {
    layout: TaffyTree<()>,
    taffy: Vec<NodeId>,
    name: Vec<Option<Box<str>>>,
    material: Vec<u32>,
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
            name: Vec::new(),
            material: Vec::new(),
        };
        // A leaf in a fresh tree has nothing to fail on.
        if let Ok(root) = tree.layout.new_leaf(root) {
            tree.append(root);
        }
        tree
    }

    /// Adds a node as the last child of `parent`. `None` when `parent` is not of this tree.
    pub fn push(&mut self, parent: UiNode, style: Style) -> Option<UiNode> {
        let parent = *self.taffy.get(parent.0 as usize)?;
        let node = self.layout.new_leaf(style).ok()?;
        self.layout.add_child(parent, node).ok()?;
        Some(self.append(node))
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

    fn append(&mut self, node: NodeId) -> UiNode {
        let index = UiNode(self.taffy.len() as u32);
        self.taffy.push(node);
        self.name.push(None);
        self.material.push(Self::NO_MATERIAL);
        index
    }
}

#[cfg(test)]
mod tests;

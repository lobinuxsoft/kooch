//! States (`:hover`, `:active`, `:focus`) on a tree built from files: the cascade runs again when
//! one changes, which is per gesture, not per frame.

use kooch_core::resource::Resources;

use crate::build::apply;
use crate::cascade::resolve;
use crate::source::UiSource;
use crate::style::UiStyleSheet;
use crate::{UiNode, UiTree, UiTrees};

/// What a tree was styled from, kept so a state change can resolve it again.
#[derive(Debug, Clone)]
pub(crate) struct Styled {
    pub source: UiSource,
    pub sheet: Option<UiStyleSheet>,
    /// The node each element became; `None` for one under a parent that failed.
    pub nodes: Vec<Option<UiNode>>,
    /// `STATE_*` bits, per element.
    pub states: Vec<u8>,
    pub dirty: bool,
}

impl UiTree {
    /// Sets or clears `STATE_*` bits on a node of a tree built from files; a tree built in code
    /// has no rules for them to select.
    pub fn set_state(&mut self, node: UiNode, bits: u8, on: bool) {
        let Some(styled) = self.styled.as_mut() else {
            return;
        };
        let Some(element) = styled.nodes.iter().position(|n| *n == Some(node)) else {
            return;
        };
        let states = &mut styled.states[element];
        let next = if on { *states | bits } else { *states & !bits };
        if next != *states {
            *states = next;
            styled.dirty = true;
        }
    }

    /// The node and its ancestors, as `:hover` and `:active` hold in CSS.
    pub fn set_state_up(&mut self, node: UiNode, bits: u8, on: bool) {
        let mut current = Some(node);
        while let Some(node) = current {
            self.set_state(node, bits, on);
            current = self.parent(node);
        }
    }

    fn restyle(&mut self) {
        let Some(styled) = self.styled.take() else {
            return;
        };
        if styled.dirty {
            let styles = resolve(&styled.source, styled.sheet.as_ref(), &styled.states);
            for (node, style) in styled.nodes.iter().zip(&styles) {
                if let Some(node) = *node {
                    self.set_style(node, style.layout.clone());
                    apply(self, node, style);
                }
            }
        }
        self.styled = Some(Styled {
            dirty: false,
            ..styled
        });
    }
}

/// After the trees are built and before they are laid out.
pub fn restyle_documents(resources: &mut Resources) {
    if let Some(trees) = resources.get_mut::<UiTrees>() {
        trees.for_each_mut(UiTree::restyle);
    }
}

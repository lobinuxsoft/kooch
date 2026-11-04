//! Elements added and removed while the game runs (#1784): a menu of items built from code.

use super::{UiNode, UiTree};
use crate::controls::{SLIDER_FILL, SLIDER_REST, SLIDER_THUMB, TOGGLE_CHECK};
use crate::source::{Element, ElementKind, NO_PARENT, UiSource};

impl UiTree {
    /// An attribute as the file, or the `<use>` that made the node, wrote it: `data-index="7"`.
    pub fn attribute(&self, node: UiNode, key: &str) -> Option<&str> {
        let styled = self.styled.as_ref()?;
        let element = styled.nodes.iter().position(|n| *n == Some(node))?;
        styled.source.elements[element]
            .attributes
            .iter()
            .find(|(own, _)| &**own == key)
            .map(|(_, value)| &**value)
    }

    /// Removes `node` and everything under it. The tree is built again without them, keeping what
    /// the game wrote — texts, values, hidden nodes, scroll — so no slot is left behind. Every
    /// `UiNode` kept from before is stale: query again by name.
    pub fn remove(&mut self, node: UiNode) -> bool {
        let Some(styled) = self.styled.as_ref() else {
            return false;
        };
        let elements = &styled.source.elements;
        let Some(start) = styled.nodes.iter().position(|n| *n == Some(node)) else {
            return false;
        };
        if elements[start].parent == NO_PARENT {
            return false;
        }
        // Parents come before children: one pass finds the subtree.
        let mut gone = vec![false; elements.len()];
        gone[start] = true;
        for index in start + 1..elements.len() {
            let parent = elements[index].parent;
            gone[index] = parent != NO_PARENT && gone[parent as usize];
        }
        let mut kept: Vec<Option<u32>> = vec![None; elements.len()];
        let mut source = UiSource::default();
        for (index, element) in elements.iter().enumerate() {
            // The build makes a control's parts again.
            if gone[index] || is_part(elements, element) {
                continue;
            }
            let mut element = element.clone();
            if element.parent != NO_PARENT {
                let Some(parent) = kept[element.parent as usize] else {
                    continue;
                };
                element.parent = parent;
            }
            kept[index] = Some(source.elements.len() as u32);
            source.elements.push(element);
        }
        let mut next = crate::build_tree(&source, styled.sheet.as_ref());
        let Some(new_nodes) = next.styled.as_ref().map(|styled| styled.nodes.clone()) else {
            return false;
        };
        for (index, slot) in kept.iter().enumerate() {
            let (Some(old), Some(Some(new))) =
                (styled.nodes[index], slot.map(|s| new_nodes[s as usize]))
            else {
                continue;
            };
            self.carry(old, &mut next, new);
        }
        next.origin = self.origin.take();
        next.shown = self.shown;
        *self = next;
        true
    }

    /// What the game wrote on `old`, written on `new` of `next`.
    fn carry(&self, old: UiNode, next: &mut UiTree, new: UiNode) {
        let i = old.0 as usize;
        if let Some(text) = &self.text[i] {
            next.write_text(new, &text.content);
        }
        if let Some(value) = self.value(old) {
            next.set_value(new, value);
        }
        if self.hidden.iter().any(|(own, _)| *own == old) {
            next.set_visible(new, false);
        }
        if self.disabled(old) {
            next.set_disabled(new, true);
        }
        next.scroll[new.0 as usize] = self.scroll[i];
    }
}

/// A part `with_parts` put under a toggle or a slider.
fn is_part(elements: &[Element], element: &Element) -> bool {
    let under_control = elements
        .get(element.parent as usize)
        .is_some_and(|parent| matches!(parent.kind, ElementKind::Toggle | ElementKind::Slider));
    under_control
        && element
            .classes
            .iter()
            .any(|class| [TOGGLE_CHECK, SLIDER_FILL, SLIDER_THUMB, SLIDER_REST].contains(&&**class))
}

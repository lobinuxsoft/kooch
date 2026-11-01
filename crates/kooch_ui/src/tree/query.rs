//! What the game asks of a document (#96 phase 7): nodes by name or class, classes and visibility.

use taffy::{Display, Style};

use super::{UiNode, UiTree};

impl UiTree {
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

    /// Every node with this class, in creation order. Classes come from files: a tree built in code
    /// has none.
    pub fn with_class<'a>(&'a self, class: &'a str) -> impl Iterator<Item = UiNode> + 'a {
        self.styled.iter().flat_map(move |styled| {
            styled
                .source
                .elements
                .iter()
                .zip(&styled.nodes)
                .filter(move |(element, _)| element.classes.iter().any(|own| &**own == class))
                .filter_map(|(_, node)| *node)
        })
    }

    pub fn has_class(&self, node: UiNode, class: &str) -> bool {
        self.with_class(class).any(|own| own == node)
    }

    /// Adds or removes a class; the sheet's rules for it apply on the next restyle, this frame.
    pub fn set_class(&mut self, node: UiNode, class: &str, on: bool) {
        let Some(styled) = self.styled.as_mut() else {
            return;
        };
        let Some(element) = styled.nodes.iter().position(|n| *n == Some(node)) else {
            return;
        };
        let classes = &mut styled.source.elements[element].classes;
        let at = classes.iter().position(|own| &**own == class);
        match (on, at) {
            (true, None) => classes.push(class.into()),
            (false, Some(at)) => {
                classes.remove(at);
            }
            _ => return,
        }
        styled.dirty = true;
    }

    /// Hides a node and everything under it, as `display: none`: it takes no space, draws nothing
    /// and neither picks nor takes focus. Showing it again restores the display it had.
    pub fn set_visible(&mut self, node: UiNode, visible: bool) {
        let Some(style) = self.style_of(node) else {
            return;
        };
        let at = self.hidden.iter().position(|(own, _)| *own == node);
        match (visible, at) {
            (false, None) => {
                self.hidden.push((node, style.display));
                self.set_style(
                    node,
                    Style {
                        display: Display::None,
                        ..style
                    },
                );
            }
            (true, Some(at)) => {
                let (_, display) = self.hidden.swap_remove(at);
                self.set_style(node, Style { display, ..style });
            }
            _ => {}
        }
    }

    /// Whether the game left the node itself visible; one under a hidden parent still is.
    pub fn visible(&self, node: UiNode) -> bool {
        !self.hidden.iter().any(|(own, _)| *own == node)
    }

    /// Hidden itself or under a hidden ancestor, as of the last layout.
    pub(crate) fn culled(&self, node: UiNode) -> bool {
        self.culled.get(node.0 as usize).copied().unwrap_or(false)
    }

    /// After a restyle, which puts the cascade's display back: that one is what showing restores.
    pub(crate) fn hide_again(&mut self) {
        for at in 0..self.hidden.len() {
            let node = self.hidden[at].0;
            let Some(style) = self.style_of(node) else {
                continue;
            };
            if style.display != Display::None {
                self.hidden[at].1 = style.display;
                self.set_style(
                    node,
                    Style {
                        display: Display::None,
                        ..style
                    },
                );
            }
        }
    }

    fn style_of(&self, node: UiNode) -> Option<Style> {
        let id = *self.taffy.get(node.0 as usize)?;
        self.layout.style(id).ok().cloned()
    }
}

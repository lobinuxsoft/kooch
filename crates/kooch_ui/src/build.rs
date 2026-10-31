//! A [`UiTree`] from a parsed `.kxml` and its sheet.

use crate::cascade::resolve;
use crate::restyle::Styled;
use crate::source::{ElementKind, NO_PARENT, UiSource};
use crate::style::{ResolvedStyle, UiStyleSheet};
use crate::{UiNode, UiTree};

/// A label's size when nothing up its chain sets `font-size`, in canvas units.
pub const DEFAULT_FONT_SIZE: f32 = 24.0;

pub fn build_tree(source: &UiSource, sheet: Option<&UiStyleSheet>) -> UiTree {
    let states = vec![0; source.elements.len()];
    let styles = resolve(source, sheet, &states);
    let mut tree: Option<UiTree> = None;
    let mut nodes: Vec<Option<UiNode>> = Vec::with_capacity(source.elements.len());
    for (element, style) in source.elements.iter().zip(&styles) {
        let node = match (element.parent, tree.as_mut()) {
            (NO_PARENT, _) => {
                tree = Some(UiTree::new(style.layout.clone()));
                Some(UiTree::ROOT)
            }
            // Its subtree goes with it: templates are phase 5.
            (_, _) if element.kind == ElementKind::Use => {
                tracing::warn!("`<use>` is skipped until templates arrive (#280 phase 5)");
                None
            }
            (parent, Some(tree)) => {
                nodes[parent as usize].and_then(|parent| tree.push(parent, style.layout.clone()))
            }
            (_, None) => None,
        };
        nodes.push(node);
        let (Some(node), Some(tree)) = (node, tree.as_mut()) else {
            continue;
        };
        if let Some(name) = &element.name {
            tree.set_name(node, name);
        }
        if let Some((_, action)) = element
            .attributes
            .iter()
            .find(|(key, _)| &**key == "on:click")
        {
            tree.set_click(node, action);
        }
        if let Some(text) = &element.text {
            tree.set_text(node, text, style.font_size.unwrap_or(DEFAULT_FONT_SIZE));
        }
        apply(tree, node, style);
    }
    let mut tree = tree.unwrap_or_else(|| UiTree::new(taffy::Style::default()));
    tree.styled = Some(Styled {
        source: source.clone(),
        sheet: sheet.cloned(),
        nodes,
        states,
        dirty: false,
    });
    tree
}

/// What a style paints on a node; layout is set where the node is made, or by the restyle.
pub(crate) fn apply(tree: &mut UiTree, node: UiNode, style: &ResolvedStyle) {
    tree.set_background(node, style.paint.background);
    tree.set_border_color(node, style.paint.border);
    tree.set_radius(node, style.paint.radius);
    tree.set_text_color(node, style.paint.text);
    tree.set_text_size(node, style.font_size.unwrap_or(DEFAULT_FONT_SIZE));
    if let Some(family) = &style.font_family {
        tree.set_font(node, family);
    }
}

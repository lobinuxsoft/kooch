//! A [`UiTree`] from a parsed `.kxml` and its sheet.

use crate::cascade::resolve;
use crate::controls::{SLIDER_FILL, SLIDER_REST, SLIDER_THUMB};
use crate::images::UiImage;
use crate::restyle::Styled;
use crate::source::{Element, NO_PARENT, UiSource};
use crate::style::{ResolvedStyle, STATE_DISABLED, UiStyleSheet};
use crate::{UiEventKind, UiMaskImage, UiNode, UiTree};

/// A label's size when nothing up its chain sets `font-size`, in canvas units.
pub const DEFAULT_FONT_SIZE: f32 = 24.0;

pub fn build_tree(source: &UiSource, sheet: Option<&UiStyleSheet>) -> UiTree {
    let source = &crate::controls::with_parts(source);
    let states: Vec<u8> = source.elements.iter().map(disabled).collect();
    let styles = resolve(source, sheet, &states);
    let mut tree: Option<UiTree> = None;
    let mut nodes: Vec<Option<UiNode>> = Vec::with_capacity(source.elements.len());
    for (element, style) in source.elements.iter().zip(&styles) {
        let node = match (element.parent, tree.as_mut()) {
            (NO_PARENT, _) => {
                tree = Some(UiTree::new(style.layout.clone()));
                Some(UiTree::ROOT)
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
        setup(
            tree,
            node,
            element,
            style.font_size.unwrap_or(DEFAULT_FONT_SIZE),
        );
        apply(tree, node, style);
    }
    let mut tree = tree.unwrap_or_else(|| UiTree::new(taffy::Style::default()));
    find_parts(&mut tree, source, &nodes);
    tree.styled = Some(Styled {
        source: source.clone(),
        sheet: sheet.cloned(),
        nodes,
        states,
        dirty: false,
    });
    // After `styled`: `:checked` is a state, and states need it.
    tree.show_values();
    tree
}

/// What an element's attributes and text make of its node; its style is the cascade's.
fn setup(tree: &mut UiTree, node: UiNode, element: &Element, font_size: f32) {
    if let Some(name) = &element.name {
        tree.set_name(node, name);
    }
    for (key, value) in &element.attributes {
        match &**key {
            "on:click" => tree.set_click(node, value),
            "on:focus" => tree
                .handlers
                .push((node, UiEventKind::Focus, value.clone())),
            "on:blur" => tree.handlers.push((node, UiEventKind::Blur, value.clone())),
            "on:change" => tree
                .handlers
                .push((node, UiEventKind::Change, value.clone())),
            "on:cancel" => tree
                .handlers
                .push((node, UiEventKind::Cancel, value.clone())),
            "tabindex" if &**value == "-1" => tree.unfocusable.push(node),
            "autofocus" => tree.autofocus = Some(node),
            "disabled" if &**value != "false" => tree.set_disabled(node, true),
            _ => tree.bind_param(node, key, value),
        }
    }
    tree.controls
        .extend(crate::controls::declared(element, node));
    if let Some(text) = &element.text {
        tree.set_text(node, text, font_size);
    }
}

pub(crate) fn disabled(element: &Element) -> u8 {
    let off = element
        .attributes
        .iter()
        .any(|(key, value)| &**key == "disabled" && &**value != "false");
    if off { STATE_DISABLED } else { 0 }
}

/// Appends an expanded `<use>` — its `<ui>` first, then the instance — as the last child of
/// `parent`, styled by the next restyle like the file's own elements. Returns the instance root.
pub(crate) fn graft(tree: &mut UiTree, parent: UiNode, instance: &UiSource) -> Option<UiNode> {
    let instance = crate::controls::with_parts(instance);
    let mut styled = tree.styled.take()?;
    let Some(at) = styled.nodes.iter().position(|n| *n == Some(parent)) else {
        tree.styled = Some(styled);
        return None;
    };
    let base = styled.source.elements.len() as u32;
    let mut root = None;
    for element in instance.elements.iter().skip(1) {
        let mut element = element.clone();
        element.parent = match element.parent {
            0 => at as u32,
            own => base + own - 1,
        };
        element.origin = styled.source.elements[at].origin;
        let node = styled.nodes[element.parent as usize]
            .and_then(|parent| tree.push(parent, taffy::Style::default()));
        if let Some(node) = node {
            setup(tree, node, &element, DEFAULT_FONT_SIZE);
            root.get_or_insert(node);
        }
        styled.states.push(disabled(&element));
        styled.nodes.push(node);
        styled.source.elements.push(element);
    }
    styled.dirty = true;
    let (source, nodes) = (styled.source.clone(), styled.nodes.clone());
    tree.styled = Some(styled);
    find_parts(tree, &source, &nodes);
    tree.show_values();
    root
}

/// What a style paints on a node; layout is set where the node is made, or by the restyle.
pub(crate) fn apply(tree: &mut UiTree, node: UiNode, style: &ResolvedStyle) {
    tree.set_background(node, style.paint.background);
    tree.set_border_color(node, style.paint.border);
    tree.set_radius(node, style.paint.radius);
    tree.set_text_color(node, style.paint.text);
    tree.set_font_share(node, style.font_share);
    // A share's size comes from the layout: writing one here would undo it at every restyle.
    if style.font_share.is_none() {
        tree.set_text_size(node, style.font_size.unwrap_or(DEFAULT_FONT_SIZE));
    }
    if let Some(family) = &style.font_family {
        tree.set_font(node, family);
    }
    let image = style.image.map(|guid| UiImage {
        guid,
        slice: style.slice,
        tint: style.tint.unwrap_or([1.0; 4]),
    });
    tree.set_image(node, image);
    let mask = style.mask.map(|guid| UiMaskImage {
        guid,
        luminance: style.mask_luminance,
    });
    tree.set_mask(node, mask);
    tree.set_fill_from(node, style.fill_from);
    tree.set_curve(node, style.curve.clone());
    tree.set_flip(node, style.flip);
    tree.set_transform(node, style.transform);
    tree.set_material(node, style.material);
    tree.style_params(node, &style.params);
}

/// Each slider's fill, thumb and rest: the parts `with_parts` appended under it, in that order.
fn find_parts(tree: &mut UiTree, source: &UiSource, nodes: &[Option<UiNode>]) {
    for control in &mut tree.controls {
        let Some(element) = nodes.iter().position(|n| *n == Some(control.node)) else {
            continue;
        };
        let parts: Vec<UiNode> = source
            .elements
            .iter()
            .zip(nodes)
            .filter(|(child, _)| {
                child.parent == element as u32
                    && child
                        .classes
                        .iter()
                        .any(|class| [SLIDER_FILL, SLIDER_THUMB, SLIDER_REST].contains(&&**class))
            })
            .filter_map(|(_, node)| *node)
            .collect();
        if let [fill, thumb, rest] = parts[..] {
            control.parts = Some([fill, thumb, rest]);
        }
    }
}

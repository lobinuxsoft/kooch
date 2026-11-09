//! A document's nodes: taffy owns structure and layout, the parallel arrays own the rest. A
//! [`UiNode`] indexes all of them, so nothing outside needs taffy's slotmap keys.

use glam::Vec2;
use kooch_core::Guid;
use taffy::{AvailableSpace, NodeId, Size, Style, TaffyTree};

use crate::materials::{NodeParam, ParamBind};
use crate::text::{Text, UiFonts, measure};
use crate::text_binds::TextBind;
use scroll::UNCLIPPED;

/// A node of one [`UiTree`]: dense from the root, which is `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UiNode(pub u32);

/// Where a node landed, in the document's logical canvas: top-left origin, y down.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UiRect {
    pub min: Vec2,
    pub size: Vec2,
}

/// One glyph to draw, in physical pixels, and where its bitmap is in the atlas.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphQuad {
    pub min: [f32; 2],
    pub size: [f32; 2],
    pub uv_min: [u16; 2],
    pub uv_size: [u16; 2],
    pub node: u32,
    /// A whole text node drawn by Vello, sampled from the vector atlas (#1880).
    pub vector: bool,
}

/// How a node is painted. Colours are sRGB, as a colour picker gives them: the frame they are
/// blended over is already display-encoded. Transparent draws nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paint {
    pub background: [f32; 4],
    pub border: [f32; 4],
    /// Corner radius, canvas units.
    pub radius: f32,
    pub text: [f32; 4],
}

impl Default for Paint {
    fn default() -> Self {
        Self {
            background: [0.0; 4],
            border: [0.0; 4],
            radius: 0.0,
            text: [1.0; 4],
        }
    }
}

/// One document's node tree. Grows only: a document that changes shape is rebuilt whole, and
/// hiding a node is `Display::None`, not removal.
#[derive(Debug)]
pub struct UiTree {
    /// The node context is the node's own index: the measure reads its text by it.
    layout: TaffyTree<u32>,
    taffy: Vec<NodeId>,
    /// `u32::MAX` for the root. Always a lower index than the child, which is what lets one forward
    /// pass turn taffy's parent-relative positions into absolute ones.
    parent: Vec<u32>,
    name: Vec<Option<Box<str>>>,
    /// The action `on:click` names (#1693).
    click: Vec<Option<Box<str>>>,
    /// Every other `on:` the file gave, sparse: few nodes have one.
    pub(crate) handlers: Vec<(UiNode, crate::UiEventKind, Box<str>)>,
    /// Nodes with an action that `tabindex="-1"` keeps out of focus.
    pub(crate) unfocusable: Vec<UiNode>,
    /// Nodes that neither click nor take focus, sparse.
    disabled: Vec<UiNode>,
    /// Nodes the game hid, sparse, with the display each had.
    hidden: Vec<(UiNode, taffy::Display)>,
    /// Hidden itself or under a hidden ancestor, per node; set by the layout.
    culled: Vec<bool>,
    /// The physical pixels on screen, `[x0, y0, x1, y1]`; `None` draws vectors whole (#1880).
    pub(crate) view: Option<[f32; 4]>,
    /// Toggles and sliders, sparse.
    pub(crate) controls: Vec<crate::controls::Control>,
    /// The node `autofocus` names: it takes focus each time the document appears.
    pub(crate) autofocus: Option<UiNode>,
    /// Drawn last frame: what tells a document that just appeared from one already up.
    pub(crate) shown: bool,
    pub(crate) material: Vec<Option<Guid>>,
    pub(crate) params: Vec<Vec<NodeParam>>,
    pub(crate) param_binds: Vec<ParamBind>,
    pub(crate) text_binds: Vec<TextBind>,
    rect: Vec<UiRect>,
    /// How far a container's children are moved up and left by scrolling, in canvas units.
    scroll: Vec<Vec2>,
    /// `[x0, y0, x1, y1]` of what can be seen of each node: every `overflow` ancestor cuts it.
    clip: Vec<[f32; 4]>,
    /// Border width from the layout, uniform: the left edge's.
    border: Vec<f32>,
    paint: Vec<Paint>,
    pub(crate) image: Vec<Option<crate::images::UiImage>>,
    pub(crate) mask: Vec<masks::UiCut>,
    pub(crate) curve: Vec<Option<curves::UiPath>>,
    flip: Vec<u32>,
    transform: Vec<transforms::UiTransform>,
    font_share: Vec<Option<f32>>,
    text: Vec<Option<Text>>,
    glyphs: Vec<GlyphQuad>,
    /// Physical pixels per canvas unit, atlas generation and text zooms the glyphs were built for.
    rastered: Option<(f32, u32, u64)>,
    /// The vector atlas's text slots this tree holds, and the generation they were taken in.
    held_texts: (u32, Vec<u64>),
    /// Size the rects were computed for; `None` once anything changed.
    laid_out: Option<Vec2>,
    /// A scroll moved rects that taffy's layout still holds: they are placed again, not laid out.
    moved: bool,
    /// The files this tree was built from; `None` for one built in code.
    pub(crate) origin: Option<crate::documents::Origin>,
    /// The cascade's inputs, for a tree built from files: what states restyle from.
    pub(crate) styled: Option<crate::restyle::Styled>,
}

// SAFETY: taffy's lengths are `!Send` only for the pointer a `calc()` length carries, and taffy is
// built without `calc`, so no length here holds one. Bevy's `UiTree` makes the same claim.
unsafe impl Send for UiTree {}
unsafe impl Sync for UiTree {}

impl UiTree {
    pub const ROOT: UiNode = UiNode(0);

    pub fn new(root: Style) -> Self {
        let mut tree = Self {
            layout: TaffyTree::new(),
            taffy: Vec::new(),
            parent: Vec::new(),
            name: Vec::new(),
            click: Vec::new(),
            handlers: Vec::new(),
            unfocusable: Vec::new(),
            disabled: Vec::new(),
            hidden: Vec::new(),
            culled: Vec::new(),
            view: None,
            controls: Vec::new(),
            autofocus: None,
            shown: false,
            material: Vec::new(),
            params: Vec::new(),
            param_binds: Vec::new(),
            text_binds: Vec::new(),
            rect: Vec::new(),
            scroll: Vec::new(),
            clip: Vec::new(),
            border: Vec::new(),
            paint: Vec::new(),
            image: Vec::new(),
            mask: Vec::new(),
            curve: Vec::new(),
            flip: Vec::new(),
            transform: Vec::new(),
            font_share: Vec::new(),
            text: Vec::new(),
            glyphs: Vec::new(),
            rastered: None,
            held_texts: (0, Vec::new()),
            laid_out: None,
            moved: false,
            origin: None,
            styled: None,
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

    /// Grows a node by `grow` against a zero basis: how a slider's parts split its track.
    pub(crate) fn set_grow(&mut self, node: UiNode, grow: f32) {
        let Some(&id) = self.taffy.get(node.0 as usize) else {
            return;
        };
        let Ok(style) = self.layout.style(id) else {
            return;
        };
        let style = Style {
            flex_grow: grow,
            flex_basis: taffy::Dimension::length(0.0),
            ..style.clone()
        };
        self.set_style(node, style);
    }

    pub fn set_style(&mut self, node: UiNode, style: Style) {
        let Some(&id) = self.taffy.get(node.0 as usize) else {
            return;
        };
        if self.layout.set_style(id, style).is_ok() {
            self.laid_out = None;
        }
    }

    /// Gives a node text, shaped with the engine font at `size` canvas units. Its content box is
    /// measured from it.
    pub fn set_text(&mut self, node: UiNode, content: &str, size: f32) {
        let Some(&id) = self.taffy.get(node.0 as usize) else {
            return;
        };
        let slot = &mut self.text[node.0 as usize];
        if slot
            .as_ref()
            .is_some_and(|text| text.content == content && text.size == size)
        {
            return;
        }
        *slot = Some(Text {
            content: content.to_owned(),
            size,
            family: slot.as_ref().and_then(|text| text.family.clone()),
            layout: None,
            origin: Vec2::ZERO,
        });
        let _ = self.layout.set_node_context(id, Some(node.0));
        let _ = self.layout.mark_dirty(id);
        self.laid_out = None;
    }

    /// Replaces a node's text, keeping the size its style gave it.
    pub fn write_text(&mut self, node: UiNode, content: &str) {
        let size = self
            .text
            .get(node.0 as usize)
            .and_then(Option::as_ref)
            .map_or(crate::build::DEFAULT_FONT_SIZE, |text| text.size);
        self.set_text(node, content, size);
    }

    /// Resizes a node's text, keeping its content. Nothing for a node without text.
    pub fn set_text_size(&mut self, node: UiNode, size: f32) {
        let Some(content) = self.text[node.0 as usize]
            .as_ref()
            .filter(|text| text.size != size)
            .map(|text| text.content.clone())
        else {
            return;
        };
        self.set_text(node, &content, size);
    }

    /// Shapes a node's text with `family` first (`"Fira Code"`), then Inter. Set after `set_text`.
    pub fn set_font(&mut self, node: UiNode, family: &str) {
        let Some(text) = self.text.get_mut(node.0 as usize).and_then(Option::as_mut) else {
            return;
        };
        if text.family.as_deref() == Some(family) {
            return;
        }
        text.family = Some(family.into());
        text.layout = None;
        if let Some(&id) = self.taffy.get(node.0 as usize) {
            let _ = self.layout.mark_dirty(id);
        }
        self.laid_out = None;
    }

    /// Lays the tree out in `available` logical units, unless it already is for that size and
    /// nothing changed since. Returns whether it ran.
    pub fn layout(&mut self, available: Size<AvailableSpace>, fonts: &mut UiFonts) -> bool {
        let ran = self.layout_once(available, fonts);
        // Resized text may move what follows it.
        if ran && self.fit_text() {
            self.layout_once(available, fonts);
        }
        ran
    }

    fn layout_once(&mut self, available: Size<AvailableSpace>, fonts: &mut UiFonts) -> bool {
        let key = Vec2::new(space_key(available.width), space_key(available.height));
        if self.taffy.is_empty() {
            return false;
        }
        if self.laid_out == Some(key) {
            if !std::mem::take(&mut self.moved) {
                return false;
            }
            self.place(fonts, false);
            self.rastered = None;
            return true;
        }
        let text = &mut self.text;
        let measured = self.layout.compute_layout_with_measure(
            self.taffy[0],
            available,
            |inputs, _, context, style| {
                taffy::compute_leaf_layout(
                    inputs,
                    style,
                    |_, _| 0.0,
                    |known, available| {
                        let Some(text) = context.and_then(|index| text[*index as usize].as_mut())
                        else {
                            return Size::ZERO;
                        };
                        let width = known.width.or(match available.width {
                            AvailableSpace::Definite(width) => Some(width),
                            AvailableSpace::MinContent => Some(0.0),
                            AvailableSpace::MaxContent => None,
                        });
                        let size = measure(text.shaped(fonts), width);
                        Size {
                            width: known.width.unwrap_or(size.x),
                            height: known.height.unwrap_or(size.y),
                        }
                    },
                )
            },
        );
        if measured.is_err() {
            return false;
        }
        self.place(fonts, true);
        self.laid_out = Some(key);
        self.moved = false;
        self.rastered = None;
        true
    }

    /// Taffy's parent-relative boxes as absolute rects, scrolled and clipped. `remeasure` after a
    /// layout, when lines may have broken at a trial width.
    fn place(&mut self, fonts: &mut UiFonts, remeasure: bool) {
        for index in 0..self.taffy.len() {
            let Ok(own) = self.layout.layout(self.taffy[index]) else {
                continue;
            };
            let (origin, culled, clip) = match self.parent[index] {
                u32::MAX => (Vec2::ZERO, false, UNCLIPPED),
                parent => {
                    let p = parent as usize;
                    let origin = self.rect[p].min - self.scroll[p];
                    (origin, self.culled[p], self.cut(p))
                }
            };
            self.culled[index] = culled || !self.visible(UiNode(index as u32));
            self.clip[index] = clip;
            // Content that shrank leaves no offset past its end.
            let limit = Vec2::new(own.scroll_width(), own.scroll_height());
            self.scroll[index] = self.scroll[index].clamp(Vec2::ZERO, limit);
            let min = origin + Vec2::new(own.location.x, own.location.y);
            let inset = Vec2::new(
                own.padding.left + own.border.left,
                own.padding.top + own.border.top,
            );
            let content = own.content_box_width();
            self.rect[index] = UiRect {
                min,
                size: Vec2::new(own.size.width, own.size.height),
            };
            self.border[index] = own.border.left;
            // The last measure may have broken lines at a trial width; the box is final now.
            if let Some(text) = self.text[index].as_mut() {
                text.origin = min + inset;
                if remeasure {
                    measure(text.shaped(fonts), Some(content));
                }
            }
        }
    }

    /// Fill colour, sRGB. The border's width is the layout's (`Style::border`).
    pub fn set_background(&mut self, node: UiNode, rgba: [f32; 4]) {
        if let Some(paint) = self.paint.get_mut(node.0 as usize) {
            paint.background = rgba;
        }
    }

    pub fn set_border_color(&mut self, node: UiNode, rgba: [f32; 4]) {
        if let Some(paint) = self.paint.get_mut(node.0 as usize) {
            paint.border = rgba;
        }
    }

    pub fn set_radius(&mut self, node: UiNode, radius: f32) {
        if let Some(paint) = self.paint.get_mut(node.0 as usize) {
            paint.radius = radius;
        }
    }

    /// Text colour, sRGB. White until set.
    pub fn set_text_color(&mut self, node: UiNode, rgba: [f32; 4]) {
        if let Some(paint) = self.paint.get_mut(node.0 as usize) {
            paint.text = rgba;
        }
    }

    pub fn paint(&self, node: UiNode) -> Option<Paint> {
        self.paint.get(node.0 as usize).copied()
    }

    pub fn border(&self, node: UiNode) -> Option<f32> {
        self.border.get(node.0 as usize).copied()
    }

    pub fn glyphs(&self) -> &[GlyphQuad] {
        &self.glyphs
    }

    pub fn rect(&self, node: UiNode) -> Option<UiRect> {
        self.rect.get(node.0 as usize).copied()
    }

    pub fn set_click(&mut self, node: UiNode, action: &str) {
        if let Some(own) = self.click.get_mut(node.0 as usize) {
            *own = Some(action.into());
        }
    }

    pub fn click(&self, node: UiNode) -> Option<&str> {
        self.click.get(node.0 as usize)?.as_deref()
    }

    /// A disabled node keeps stopping the pointer, so the click does not fall through to the game,
    /// but sends nothing and takes no focus. Selects `:disabled`.
    pub fn set_disabled(&mut self, node: UiNode, on: bool) {
        self.disabled.retain(|&own| own != node);
        if on {
            self.disabled.push(node);
        }
        self.set_state(node, crate::style::STATE_DISABLED, on);
    }

    pub fn disabled(&self, node: UiNode) -> bool {
        self.disabled.contains(&node)
    }

    pub fn parent(&self, node: UiNode) -> Option<UiNode> {
        let parent = *self.parent.get(node.0 as usize)?;
        (parent != u32::MAX).then_some(UiNode(parent))
    }

    /// The topmost node under `point` that stops the pointer: one with an action or a visible
    /// background. A transparent layout panel lets it through to the game.
    pub fn pick(&self, point: Vec2) -> Option<UiNode> {
        self.pick_masked(point, None)
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
        self.click.push(None);
        self.material.push(None);
        self.params.push(Vec::new());
        self.rect.push(UiRect::default());
        self.scroll.push(Vec2::ZERO);
        self.clip.push(UNCLIPPED);
        self.border.push(0.0);
        self.paint.push(Paint::default());
        self.image.push(None);
        self.mask.push(masks::UiCut::default());
        self.curve.push(None);
        self.flip.push(0);
        self.transform.push(transforms::UiTransform::NONE);
        self.font_share.push(None);
        self.text.push(None);
        self.culled.push(false);
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

pub(crate) mod curves;
mod fit;
pub(crate) mod flips;
mod instances;
pub(crate) mod masks;
mod query;
mod raster;
mod scroll;
#[cfg(test)]
mod tests;
pub(crate) mod transforms;
#[cfg(feature = "vello")]
mod vector_text;

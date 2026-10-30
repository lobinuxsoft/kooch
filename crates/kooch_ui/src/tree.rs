//! A document's nodes: taffy owns structure and layout, the parallel arrays own the rest. A
//! [`UiNode`] indexes all of them, so nothing outside needs taffy's slotmap keys.

use std::hash::{Hash, Hasher};

use glam::Vec2;
use taffy::{AvailableSpace, NodeId, Size, Style, TaffyTree};

use crate::atlas::{GlyphAtlas, GlyphKey, subpixel};
use crate::text::{Text, UiFonts, measure};

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
    material: Vec<u32>,
    rect: Vec<UiRect>,
    /// Border width from the layout, uniform: the left edge's.
    border: Vec<f32>,
    paint: Vec<Paint>,
    text: Vec<Option<Text>>,
    glyphs: Vec<GlyphQuad>,
    /// Physical pixels per canvas unit and atlas generation the glyphs were built for.
    rastered: Option<(f32, u32)>,
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
            border: Vec::new(),
            paint: Vec::new(),
            text: Vec::new(),
            glyphs: Vec::new(),
            rastered: None,
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
        let key = Vec2::new(space_key(available.width), space_key(available.height));
        if self.laid_out == Some(key) || self.taffy.is_empty() {
            return false;
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
        for index in 0..self.taffy.len() {
            let Ok(own) = self.layout.layout(self.taffy[index]) else {
                continue;
            };
            let origin = match self.parent[index] {
                u32::MAX => Vec2::ZERO,
                parent => self.rect[parent as usize].min,
            };
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
                measure(text.shaped(fonts), Some(content));
            }
        }
        self.laid_out = Some(key);
        self.rastered = None;
        true
    }

    /// Rebuilds the glyph quads when the layout, the scale or the atlas changed. `scale` is
    /// physical pixels per canvas unit. Returns whether it ran.
    pub fn rasterize(&mut self, scale: f32, fonts: &mut UiFonts, atlas: &mut GlyphAtlas) -> bool {
        if self.rastered == Some((scale, atlas.generation())) {
            return false;
        }
        // Twice at most: a full atlas clears itself mid-pass, and what was built before points at
        // space it gave away.
        for _ in 0..2 {
            let generation = atlas.generation();
            self.glyphs.clear();
            for (index, text) in self.text.iter_mut().enumerate() {
                let Some(text) = text.as_mut() else {
                    continue;
                };
                let origin = text.origin;
                let layout = text.shaped(fonts);
                push_glyphs(
                    layout,
                    origin,
                    scale,
                    index as u32,
                    &mut fonts.scale,
                    atlas,
                    &mut self.glyphs,
                );
            }
            if atlas.generation() == generation {
                self.rastered = Some((scale, generation));
                return true;
            }
        }
        true
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
        self.border.push(0.0);
        self.paint.push(Paint::default());
        self.text.push(None);
        index
    }
}

fn push_glyphs(
    layout: &parley::Layout<()>,
    origin: Vec2,
    scale: f32,
    node: u32,
    context: &mut swash::scale::ScaleContext,
    atlas: &mut GlyphAtlas,
    out: &mut Vec<GlyphQuad>,
) {
    for line in layout.lines() {
        for item in line.items() {
            let parley::PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let shaped = run.run();
            let font = shaped.font();
            let Some(font_ref) = swash::FontRef::from_index(font.data.data(), font.index as usize)
            else {
                continue;
            };
            let coords: Vec<i16> = shaped
                .normalized_coords()
                .iter()
                .map(|coord| coord.to_bits())
                .collect();
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            coords.hash(&mut hasher);
            let coords_hash = hasher.finish();
            let size = (shaped.font_size() * scale * 64.0).round() as u32;
            for glyph in run.positioned_glyphs() {
                let x = (origin.x + glyph.x) * scale;
                let y = ((origin.y + glyph.y) * scale).round();
                let (pixel, bin) = subpixel(x);
                let key = GlyphKey {
                    font: font.data.id(),
                    index: font.index,
                    glyph: glyph.id as u16,
                    size,
                    subpixel: bin,
                    coords: coords_hash,
                };
                let Some(slot) = atlas.glyph(context, font_ref, &coords, key) else {
                    continue;
                };
                out.push(GlyphQuad {
                    min: [pixel + slot.left as f32, y - slot.top as f32],
                    size: [slot.size[0] as f32, slot.size[1] as f32],
                    uv_min: slot.min,
                    uv_size: slot.size,
                    node,
                });
            }
        }
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

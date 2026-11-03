//! Turning laid-out Overlay documents into the frame's [`UiDrawList`].

use glam::Vec2;
use kooch_core::resource::Resources;
use kooch_ecs::query::Query;
use kooch_ecs::{AtlasUpload, Entity, QUAD_GLYPH, QUAD_IMAGE, UiDrawList, UiQuad, ViewSize};

use crate::layout::CANVAS_HEIGHT;
use crate::{GlyphAtlas, MODE_OVERLAY, UiDocument, UiImage, UiNode, UiTree, UiTrees};

/// Every Overlay document, in spawn order; within one, a node's panel, image and text, parents
/// first. World documents draw in phase 5.
pub fn draw_documents(resources: &mut Resources) {
    let physical = resources.get::<ViewSize>().copied().unwrap_or_default().0;
    let scale = physical.y.max(1) as f32 / CANVAS_HEIGHT;
    let screen = [0.0, 0.0, physical.x as f32, physical.y as f32];
    let mut overlays: Vec<Entity> = Vec::new();
    Query::<&UiDocument>::new(resources).for_each_entity(|entity, document| {
        if document.mode == MODE_OVERLAY && !document.hidden {
            overlays.push(entity);
        }
    });
    let mut list = resources.remove::<UiDrawList>().unwrap_or_default();
    list.quads.clear();
    if let (Some(trees), Some(atlas)) = (resources.get::<UiTrees>(), resources.get::<GlyphAtlas>())
    {
        for entity in overlays {
            if let Some(tree) = trees.get(entity) {
                push_tree(tree, atlas, scale, screen, &mut list.quads);
            }
        }
    }
    if let Some(atlas) = resources.get_mut::<GlyphAtlas>() {
        upload(atlas, &mut list);
    }
    resources.insert(list);
}

/// One tree over a target of `physical` pixels at `scale` pixels per canvas unit, its atlas's
/// changes included: a tree that belongs to no entity, as the editor's.
pub fn draw_tree(
    tree: &UiTree,
    atlas: &mut GlyphAtlas,
    scale: f32,
    physical: glam::UVec2,
    list: &mut UiDrawList,
) {
    let screen = [0.0, 0.0, physical.x as f32, physical.y as f32];
    list.quads.clear();
    push_tree(tree, atlas, scale, screen, &mut list.quads);
    upload(atlas, list);
}

fn upload(atlas: &mut GlyphAtlas, list: &mut UiDrawList) {
    list.atlas_side = atlas.side();
    // Grown over one not uploaded yet, not replacing it: a frame the renderer skipped would
    // otherwise lose its glyphs.
    if let Some(dirty) = atlas.take_dirty() {
        let rect = match list.atlas_upload.take() {
            Some(pending) => union(pending.rect, dirty),
            None => dirty,
        };
        list.atlas_upload = Some(AtlasUpload {
            rect,
            texels: region(atlas, rect),
        });
    }
}

fn push_tree(
    tree: &UiTree,
    atlas: &GlyphAtlas,
    scale: f32,
    screen: [f32; 4],
    out: &mut Vec<UiQuad>,
) {
    let glyphs = tree.glyphs();
    let mut next = 0;
    for index in 0..tree.len() as u32 {
        let node = UiNode(index);
        // Glyphs built for a node not drawn now are passed over, so `next` stays in step.
        while next < glyphs.len() && glyphs[next].node < index {
            next += 1;
        }
        if tree.culled(node) || tree.clipped(node) {
            continue;
        }
        let (Some(rect), Some(paint), Some(border), Some(cut)) = (
            tree.rect(node),
            tree.paint(node),
            tree.border(node),
            tree.clip(node),
        ) else {
            continue;
        };
        let clip = [
            (cut[0] * scale).max(screen[0]),
            (cut[1] * scale).max(screen[1]),
            (cut[2] * scale).min(screen[2]),
            (cut[3] * scale).min(screen[3]),
        ];
        if paint.background[3] > 0.0 || (border > 0.0 && paint.border[3] > 0.0) {
            // Rounded to whole physical pixels here: the layout is in canvas units on purpose.
            let min = (rect.min * scale).round();
            let max = ((rect.min + rect.size) * scale).round();
            out.push(UiQuad {
                min: min.into(),
                size: (max - min).into(),
                clip,
                color: paint.background,
                border_color: paint.border,
                radius: paint.radius * scale,
                border: (border * scale).round(),
                ..UiQuad::default()
            });
        }
        if let Some(image) = tree.image(node)
            && let Some(packed) = atlas.image(image.guid)
        {
            let min = (rect.min * scale).round();
            let max = ((rect.min + rect.size) * scale).round();
            push_image(image, packed.rect, min, max, scale, clip, out);
        }
        while next < glyphs.len() && glyphs[next].node == index {
            let glyph = glyphs[next];
            out.push(UiQuad {
                min: glyph.min,
                size: glyph.size,
                clip,
                color: paint.text,
                uv_min: [glyph.uv_min[0] as u32, glyph.uv_min[1] as u32],
                uv_size: [glyph.uv_size[0] as u32, glyph.uv_size[1] as u32],
                flags: QUAD_GLYPH,
                ..UiQuad::default()
            });
            next += 1;
        }
        push_thumb(tree, node, rect, paint.text, scale, clip, out);
    }
}

/// Thumb width in canvas units, drawn over the content's edge: taffy reserves no gutter.
const THUMB: f32 = 6.0;

/// A thumb on each axis a `scroll` node's content overflows, sized by how much of it shows.
fn push_thumb(
    tree: &UiTree,
    node: UiNode,
    rect: crate::UiRect,
    color: [f32; 4],
    scale: f32,
    clip: [f32; 4],
    out: &mut Vec<UiQuad>,
) {
    let Some(extent) = tree.scroll_extent(node) else {
        return;
    };
    let offset = tree.scroll(node);
    let color = [color[0], color[1], color[2], color[3] * 0.35];
    for axis in 0..2 {
        if !(extent[axis] > 0.0) {
            continue;
        }
        let span = rect.size[axis];
        let length = (span * span / (span + extent[axis])).max(THUMB * 2.0);
        let along = (span - length) * offset[axis] / extent[axis];
        let mut min = rect.min;
        let mut size = Vec2::splat(THUMB);
        min[axis] += along;
        size[axis] = length;
        min[1 - axis] += rect.size[1 - axis] - THUMB;
        out.push(UiQuad {
            min: (min * scale).round().into(),
            size: (size * scale).round().into(),
            clip,
            color,
            radius: THUMB * scale / 2.0,
            ..UiQuad::default()
        });
    }
}

/// Nine quads when sliced, so the corners keep their texels' size and the middle stretches; one
/// when not. A slice wider than the rect is squeezed to fit, as CSS's `border-image` does.
fn push_image(
    image: UiImage,
    [u0, v0, u1, v1]: [u32; 4],
    min: Vec2,
    max: Vec2,
    scale: f32,
    screen: [f32; 4],
    out: &mut Vec<UiQuad>,
) {
    let [top, right, bottom, left] = image.slice.map(|edge| edge.max(0.0));
    let texel_w = (u1 - u0) as f32;
    let texel_h = (v1 - v0) as f32;
    let (left, right) = squeeze(left, right, texel_w);
    let (top, bottom) = squeeze(top, bottom, texel_h);
    let (screen_left, screen_right) = squeeze(left * scale, right * scale, max.x - min.x);
    let (screen_top, screen_bottom) = squeeze(top * scale, bottom * scale, max.y - min.y);
    let xs = [min.x, min.x + screen_left, max.x - screen_right, max.x];
    let ys = [min.y, min.y + screen_top, max.y - screen_bottom, max.y];
    let us = [u0, u0 + left as u32, u1 - right as u32, u1];
    let vs = [v0, v0 + top as u32, v1 - bottom as u32, v1];
    for row in 0..3 {
        for column in 0..3 {
            let size = [xs[column + 1] - xs[column], ys[row + 1] - ys[row]];
            if size[0] <= 0.0 || size[1] <= 0.0 {
                continue;
            }
            out.push(UiQuad {
                min: [xs[column], ys[row]],
                size,
                clip: screen,
                color: image.tint,
                uv_min: [us[column], vs[row]],
                uv_size: [us[column + 1] - us[column], vs[row + 1] - vs[row]],
                flags: QUAD_IMAGE,
                ..UiQuad::default()
            });
        }
    }
}

/// Two edges scaled down together until they fit in `span`.
fn squeeze(a: f32, b: f32, span: f32) -> (f32, f32) {
    let total = a + b;
    if total > span && total > 0.0 {
        let k = span / total;
        (a * k, b * k)
    } else {
        (a, b)
    }
}

fn region(atlas: &GlyphAtlas, [x0, y0, x1, y1]: [u32; 4]) -> Vec<u8> {
    let side = atlas.side() as usize;
    let mut texels = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
    for row in y0 as usize..y1 as usize {
        let start = (row * side + x0 as usize) * 4;
        texels.extend_from_slice(&atlas.pixels()[start..start + (x1 - x0) as usize * 4]);
    }
    texels
}

fn union(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

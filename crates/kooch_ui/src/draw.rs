//! Turning laid-out Overlay documents into the frame's [`UiDrawList`].

use kooch_core::resource::Resources;
use kooch_ecs::query::Query;
use kooch_ecs::{AtlasUpload, Entity, QUAD_GLYPH, UiDrawList, UiQuad, ViewSize};

use crate::layout::CANVAS_HEIGHT;
use crate::{GlyphAtlas, MODE_OVERLAY, UiDocument, UiNode, UiTree, UiTrees};

/// Every Overlay document, in spawn order; within one, a node's panel and then its text, parents
/// first. World documents draw in phase 5.
pub fn draw_documents(resources: &mut Resources) {
    let physical = resources.get::<ViewSize>().copied().unwrap_or_default().0;
    let scale = physical.y.max(1) as f32 / CANVAS_HEIGHT;
    let screen = [0.0, 0.0, physical.x as f32, physical.y as f32];
    let mut overlays: Vec<Entity> = Vec::new();
    Query::<&UiDocument>::new(resources).for_each_entity(|entity, document| {
        if document.mode == MODE_OVERLAY {
            overlays.push(entity);
        }
    });
    let mut list = resources.remove::<UiDrawList>().unwrap_or_default();
    list.quads.clear();
    if let Some(trees) = resources.get::<UiTrees>() {
        for entity in overlays {
            if let Some(tree) = trees.get(entity) {
                push_tree(tree, scale, screen, &mut list.quads);
            }
        }
    }
    if let Some(atlas) = resources.get_mut::<GlyphAtlas>() {
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
    resources.insert(list);
}

fn push_tree(tree: &UiTree, scale: f32, screen: [f32; 4], out: &mut Vec<UiQuad>) {
    let glyphs = tree.glyphs();
    let mut next = 0;
    for index in 0..tree.len() as u32 {
        let node = UiNode(index);
        let (Some(rect), Some(paint), Some(border)) =
            (tree.rect(node), tree.paint(node), tree.border(node))
        else {
            continue;
        };
        if paint.background[3] > 0.0 || (border > 0.0 && paint.border[3] > 0.0) {
            // Rounded to whole physical pixels here: the layout is in canvas units on purpose.
            let min = (rect.min * scale).round();
            let max = ((rect.min + rect.size) * scale).round();
            out.push(UiQuad {
                min: min.into(),
                size: (max - min).into(),
                clip: screen,
                color: paint.background,
                border_color: paint.border,
                radius: paint.radius * scale,
                border: (border * scale).round(),
                ..UiQuad::default()
            });
        }
        while next < glyphs.len() && glyphs[next].node == index {
            let glyph = glyphs[next];
            out.push(UiQuad {
                min: glyph.min,
                size: glyph.size,
                clip: screen,
                color: paint.text,
                uv_min: [glyph.uv_min[0] as u32, glyph.uv_min[1] as u32],
                uv_size: [glyph.uv_size[0] as u32, glyph.uv_size[1] as u32],
                flags: QUAD_GLYPH,
                ..UiQuad::default()
            });
            next += 1;
        }
    }
}

fn region(atlas: &GlyphAtlas, [x0, y0, x1, y1]: [u32; 4]) -> Vec<u8> {
    let side = atlas.side() as usize;
    let mut texels = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
    for row in y0 as usize..y1 as usize {
        texels
            .extend_from_slice(&atlas.pixels()[row * side + x0 as usize..row * side + x1 as usize]);
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

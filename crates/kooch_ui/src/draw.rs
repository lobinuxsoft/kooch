//! Turning laid-out Overlay documents into the frame's [`UiDrawList`].

use glam::{Mat3, Vec2};
use kooch_core::resource::Resources;
use kooch_ecs::query::Query;
use kooch_ecs::{
    AtlasUpload, Entity, MASK_VECTOR, QUAD_GLYPH, QUAD_IMAGE, QUAD_VECTOR, UiCurve, UiDrawList,
    UiMask, UiMaterial, UiMirror, UiParam, UiQuad, UiShader, ViewSize,
};

use crate::layout::CANVAS_HEIGHT;
use crate::{
    AtlasImage, FLIP_X, FLIP_Y, GlyphAtlas, MODE_OVERLAY, UiDocument, UiImage, UiNode, UiShaders,
    UiTree, UiTrees,
};

/// Bounds of what the viewport cuts where it lands. Finite: `UNCLIPPED` would not interpolate.
const ANYWHERE: [f32; 4] = [-1.0e6, -1.0e6, 1.0e6, 1.0e6];

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
    clear(&mut list);
    if let (Some(trees), Some(atlas), Some(shaders)) = (
        resources.get::<UiTrees>(),
        resources.get::<GlyphAtlas>(),
        resources.get::<UiShaders>(),
    ) {
        for entity in overlays {
            if let Some(tree) = trees.get(entity) {
                push_tree(tree, atlas, shaders, scale, screen, &mut list);
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
    shaders: &UiShaders,
    scale: f32,
    physical: glam::UVec2,
    list: &mut UiDrawList,
) {
    let screen = [0.0, 0.0, physical.x as f32, physical.y as f32];
    clear(list);
    push_tree(tree, atlas, shaders, scale, screen, list);
    upload(atlas, list);
}

fn clear(list: &mut UiDrawList) {
    list.quads.clear();
    list.masks.clear();
    list.curves.clear();
    list.mirrors.clear();
    list.materials.clear();
    list.params.clear();
    list.shaders.clear();
}

fn upload(atlas: &mut GlyphAtlas, list: &mut UiDrawList) {
    list.atlas_side = atlas.side();
    list.vector_side = atlas.vector_side();
    list.vectors = atlas.vectors();
    // Appended, as the glyph upload is grown: the renderer takes them when it draws.
    let (sources, texts, fonts) = atlas.take_sources();
    list.vector_sources.extend(sources);
    list.vector_texts.extend(texts);
    list.vector_fonts.extend(fonts);
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
    shaders: &UiShaders,
    scale: f32,
    screen: [f32; 4],
    list: &mut UiDrawList,
) {
    let glyphs = tree.glyphs();
    let mut next = 0;
    // What each node's children are cut by; a parent always comes before them.
    let mut inner = vec![0; tree.len()];
    // The curve each node's children bend along, the same way.
    let mut bent = vec![0; tree.len()];
    // The flips and transforms around each node's children, the same way.
    let mut turned = vec![Turn::NONE; tree.len()];
    for index in 0..tree.len() as u32 {
        let node = UiNode(index);
        let around = tree.parent(node).map_or(0, |up| inner[up.0 as usize]);
        let (own, within) = push_masks(tree, node, atlas, scale, around, &mut list.masks);
        inner[index as usize] = within;
        let curve = tree.parent(node).map_or(0, |up| bent[up.0 as usize]);
        let shaped = push_curve(tree, node, scale, &mut list.curves);
        bent[index as usize] = shaped.unwrap_or(curve);
        let around = tree.parent(node).map(|up| turned[up.0 as usize]);
        let own_turn = push_turn(
            tree,
            node,
            scale,
            around.unwrap_or(Turn::NONE),
            &mut list.mirrors,
        );
        turned[index as usize] = match shaped {
            Some(_) => past_curve(own_turn, &mut list.mirrors),
            None => own_turn,
        };
        let first = list.quads.len();
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
        // The clip is tested in the layout: what moves when drawn may land on screen from off it.
        let bounds = if curve != 0 || own_turn.index != 0 {
            ANYWHERE
        } else {
            screen
        };
        let clip = [
            (cut[0] * scale).max(bounds[0]),
            (cut[1] * scale).max(bounds[1]),
            (cut[2] * scale).min(bounds[2]),
            (cut[3] * scale).min(bounds[3]),
        ];
        let shaded = tree.material(node).and_then(|guid| {
            let source = shaders.source(guid)?;
            Some(UiShader {
                guid,
                source: source.clone(),
            })
        });
        let image = tree
            .image(node)
            .zip(tree.rect(node))
            .and_then(|(image, rect)| {
                let shown = crate::images::shown_size(rect.size * tree.zoom_of(node), scale);
                Some((image, atlas.image(image.guid, shown)?))
            });
        let plain = shaded.is_none();
        if let Some(shader) = shaded {
            // One quad over the whole rect: its uv has to span the node, so a slice does not apply.
            let min = (rect.min * scale).round();
            let max = ((rect.min + rect.size) * scale).round();
            let [u0, v0, u1, v1] = image.map_or([0; 4], |(_, packed)| packed.rect);
            let quad = UiQuad {
                min: min.into(),
                size: (max - min).into(),
                clip,
                color: paint.background,
                border_color: paint.border,
                radius: paint.radius * scale,
                border: (border * scale).round(),
                uv_min: [u0, v0],
                uv_size: [u1 - u0, v1 - v0],
                flags: image.map_or(0, |(_, packed)| image_flags(packed)),
                ..UiQuad::default()
            };
            push_material(tree, node, shader, quad, list);
        } else if paint.background[3] > 0.0 || (border > 0.0 && paint.border[3] > 0.0) {
            // Rounded to whole physical pixels here: the layout is in canvas units on purpose.
            let min = (rect.min * scale).round();
            let max = ((rect.min + rect.size) * scale).round();
            list.quads.push(UiQuad {
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
        let min = (rect.min * scale).round();
        let max = ((rect.min + rect.size) * scale).round();
        let cropped = image
            .filter(|(image, _)| plain && image.slice_in(atlas) == [0.0; 4])
            .and_then(|(image, _)| {
                let crop = crate::crops::crop_of(tree, node, scale)?;
                Some((image, crop, atlas.crop(image.guid, crop)))
            });
        match cropped {
            // Off the view: nothing of it is drawn.
            Some((_, crop, _)) if crop.size.contains(&0) => {}
            Some((image, crop, Some(packed))) => {
                push_crop(image, packed, crop, min, clip, &mut list.quads);
            }
            _ => {
                if let Some((image, packed)) = image.filter(|_| plain) {
                    let image = UiImage {
                        slice: Some(image.slice_in(atlas)),
                        ..image
                    };
                    push_image(image, packed, min, max, scale, clip, &mut list.quads);
                }
            }
        }
        let mut set = list.quads.len()..list.quads.len();
        while next < glyphs.len() && glyphs[next].node == index {
            let glyph = glyphs[next];
            if glyph.bent {
                set.end = list.quads.len() + 1;
            }
            list.quads.push(UiQuad {
                min: glyph.min,
                size: glyph.size,
                clip,
                color: paint.text,
                uv_min: [glyph.uv_min[0] as u32, glyph.uv_min[1] as u32],
                uv_size: [glyph.uv_size[0] as u32, glyph.uv_size[1] as u32],
                flags: match glyph.vector {
                    true => QUAD_IMAGE | QUAD_VECTOR,
                    false => QUAD_GLYPH,
                },
                ..UiQuad::default()
            });
            next += 1;
        }
        push_thumb(tree, node, rect, paint.text, scale, clip, &mut list.quads);
        for quad in &mut list.quads[first..] {
            quad.mask = own;
            quad.curve = curve;
            quad.mirror = own_turn.index;
        }
        // Glyphs already set along the curve: bent again they would turn twice.
        for quad in list.quads.get_mut(set).into_iter().flatten() {
            quad.curve = 0;
        }
    }
}

/// The masks `node` adds inside `around`. Gives back what cuts the node, which its image mask does,
/// and what cuts its children, which its rounded `overflow` does on top.
fn push_masks(
    tree: &UiTree,
    node: UiNode,
    atlas: &GlyphAtlas,
    scale: f32,
    around: u32,
    masks: &mut Vec<UiMask>,
) -> (u32, u32) {
    let mut own = around;
    let image = tree
        .mask(node)
        .zip(tree.rect(node))
        .and_then(|(mask, rect)| {
            let shown = crate::images::shown_size(rect.size * tree.zoom_of(node), scale);
            Some((mask, atlas.image(mask.guid, shown)?))
        });
    let fill_from = tree.fill_from(node);
    if let (true, Some(rect), Some(paint)) = (
        image.is_some() || fill_from != 0,
        tree.rect(node),
        tree.paint(node),
    ) {
        let [x0, y0, x1, y1] = image.map_or([0; 4], |(_, image)| image.rect);
        masks.push(UiMask {
            min: (rect.min * scale).into(),
            size: (rect.size * scale).into(),
            radius: paint.radius * scale,
            parent: around,
            uv_min: [x0, y0],
            uv_size: [x1 - x0, y1 - y0],
            luminance: image.map_or(0, |(mask, packed)| {
                u32::from(mask.luminance) | if packed.vector { MASK_VECTOR } else { 0 }
            }),
            fill_from,
            fill: tree.param(node, "fill").map_or(1.0, |value| value[0]),
            pad: 0.0,
        });
        own = masks.len() as u32;
    }
    let Some(shape) = tree.rounded(node) else {
        return (own, own);
    };
    masks.push(UiMask {
        min: (shape.min * scale).into(),
        size: (shape.size * scale).into(),
        radius: shape.radius * scale,
        parent: own,
        ..UiMask::default()
    });
    (own, masks.len() as u32)
}

/// `node`'s curve in pixels, as an index plus one.
fn push_curve(tree: &UiTree, node: UiNode, scale: f32, curves: &mut Vec<UiCurve>) -> Option<u32> {
    let line = tree.line(node)?;
    let rect = tree.rect(node)?;
    curves.push(UiCurve {
        points: line.map(|point| (point * scale).into()),
        left: rect.min.x * scale,
        width: rect.size.x * scale,
        middle: (rect.min.y + rect.size.y * 0.5) * scale,
        closed: tree.curve_closed(node).into(),
    });
    Some(curves.len() as u32)
}

/// What moves a node's quads: `before` its curve, `after` it, and their index in the list plus one.
#[derive(Clone, Copy)]
struct Turn {
    before: Mat3,
    after: Mat3,
    index: u32,
}

impl Turn {
    const NONE: Self = Self {
        before: Mat3::IDENTITY,
        after: Mat3::IDENTITY,
        index: 0,
    };

    fn pushed(before: Mat3, after: Mat3, mirrors: &mut Vec<UiMirror>) -> Self {
        let padded = |matrix: Mat3| matrix.to_cols_array_2d().map(|[x, y, z]| [x, y, z, 0.0]);
        mirrors.push(UiMirror {
            before: padded(before),
            after: padded(after),
        });
        Self {
            before,
            after,
            index: mirrors.len() as u32,
        }
    }
}

/// `node`'s flip in physical pixels, about its centre.
fn flip_matrix(tree: &UiTree, node: UiNode, scale: f32) -> Option<Mat3> {
    let flip = tree.flip_of(node);
    let rect = tree.rect(node).filter(|_| flip != 0)?;
    // Twice the centre, whole: a half pixel here would blur every edge inside.
    let far = ((rect.min * 2.0 + rect.size) * scale).round();
    let (sx, mx) = if flip & FLIP_X != 0 {
        (-1.0, far.x)
    } else {
        (1.0, 0.0)
    };
    let (sy, my) = if flip & FLIP_Y != 0 {
        (-1.0, far.y)
    } else {
        (1.0, 0.0)
    };
    Some(Mat3::from_translation(Vec2::new(mx, my)) * Mat3::from_scale(Vec2::new(sx, sy)))
}

/// `around` with `node`'s flip and transform inside it.
fn push_turn(
    tree: &UiTree,
    node: UiNode,
    scale: f32,
    around: Turn,
    mirrors: &mut Vec<UiMirror>,
) -> Turn {
    let flip = flip_matrix(tree, node, scale);
    let pixels = Mat3::from_scale(Vec2::splat(scale));
    let transform = tree
        .turn_of(node)
        .map(|turn| pixels * turn * pixels.inverse());
    if flip.is_none() && transform.is_none() {
        return around;
    }
    let own = transform.unwrap_or(Mat3::IDENTITY) * flip.unwrap_or(Mat3::IDENTITY);
    Turn::pushed(around.before * own, around.after, mirrors)
}

/// What is inside a curve bends first: the turns around it move the bent result.
fn past_curve(turn: Turn, mirrors: &mut Vec<UiMirror>) -> Turn {
    if turn.before == Mat3::IDENTITY {
        return turn;
    }
    Turn::pushed(Mat3::IDENTITY, turn.after * turn.before, mirrors)
}

/// `quad` as drawn by `shader`, with the node's values.
fn push_material(
    tree: &UiTree,
    node: UiNode,
    shader: UiShader,
    mut quad: UiQuad,
    list: &mut UiDrawList,
) {
    let index = list
        .shaders
        .iter()
        .position(|own| own.guid == shader.guid)
        .unwrap_or_else(|| {
            list.shaders.push(shader);
            list.shaders.len() - 1
        });
    let first = list.params.len() as u32;
    list.params
        .extend(tree.params[node.0 as usize].iter().map(|param| UiParam {
            name: param.name.clone(),
            value: param.value,
        }));
    quad.material = list.materials.len() as u32;
    list.materials.push(UiMaterial {
        quad: list.quads.len() as u32,
        shader: index as u32,
        params: [first, list.params.len() as u32 - first],
    });
    list.quads.push(quad);
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

/// One quad over the part of the node `crop` holds, at its own pixels (#1880).
fn push_crop(
    image: UiImage,
    packed: AtlasImage,
    crop: crate::crops::Crop,
    min: Vec2,
    clip: [f32; 4],
    out: &mut Vec<UiQuad>,
) {
    let [u0, v0, u1, v1] = packed.rect;
    out.push(UiQuad {
        min: [min.x + crop.from[0] as f32, min.y + crop.from[1] as f32],
        size: crop.size.map(|v| v as f32),
        clip,
        color: image.tint,
        uv_min: [u0, v0],
        uv_size: [u1 - u0, v1 - v0],
        flags: image_flags(packed),
        ..UiQuad::default()
    });
}

/// Which texture the quad samples.
fn image_flags(packed: AtlasImage) -> u32 {
    match packed.vector {
        true => QUAD_IMAGE | QUAD_VECTOR,
        false => QUAD_IMAGE,
    }
}

/// Nine quads when sliced, so the corners keep their size and the middle stretches; one when not.
/// A slice wider than the rect is squeezed to fit, as CSS's `border-image` does. The slice is in
/// the file's units, which `density` turns into texels.
fn push_image(
    image: UiImage,
    packed: AtlasImage,
    min: Vec2,
    max: Vec2,
    scale: f32,
    screen: [f32; 4],
    out: &mut Vec<UiQuad>,
) {
    let [u0, v0, u1, v1] = packed.rect;
    let [dx, dy] = packed.density.map(|d| d.max(f32::EPSILON));
    let [top, right, bottom, left] = image.slice.unwrap_or_default().map(|edge| edge.max(0.0));
    let (left, right) = squeeze(left, right, (u1 - u0) as f32 / dx);
    let (top, bottom) = squeeze(top, bottom, (v1 - v0) as f32 / dy);
    let (screen_left, screen_right) = squeeze(left * scale, right * scale, max.x - min.x);
    let (screen_top, screen_bottom) = squeeze(top * scale, bottom * scale, max.y - min.y);
    let xs = [min.x, min.x + screen_left, max.x - screen_right, max.x];
    let ys = [min.y, min.y + screen_top, max.y - screen_bottom, max.y];
    let us = [u0, u0 + (left * dx) as u32, u1 - (right * dx) as u32, u1];
    let vs = [v0, v0 + (top * dy) as u32, v1 - (bottom * dy) as u32, v1];
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
                flags: image_flags(packed),
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

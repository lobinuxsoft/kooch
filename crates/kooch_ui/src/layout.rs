//! Laying every document out in the space its mode gives it.

use kooch_core::resource::Resources;
use kooch_ecs::query::Query;
use kooch_ecs::{Entity, ViewSize};
use taffy::{AvailableSpace, Size};

use crate::{GlyphAtlas, MODE_OVERLAY, UiDocument, UiFonts, UiTrees};

/// Height of the logical canvas an Overlay is laid out in: the same UI fills the same share of a
/// handheld and a monitor, with no OS scale factor involved.
pub const CANVAS_HEIGHT: f32 = 1080.0;

/// The logical canvas for a view of `physical` pixels.
pub fn canvas(physical: glam::UVec2) -> glam::Vec2 {
    let height = physical.y.max(1) as f32;
    glam::Vec2::new(physical.x as f32 * CANVAS_HEIGHT / height, CANVAS_HEIGHT)
}

/// Overlay documents take the canvas; World ones their content size, until phase 5 sizes them.
/// Glyphs are rasterized at the view's physical size, so text is sharp at any resolution.
pub fn layout_documents(resources: &mut Resources) {
    let physical = resources.get::<ViewSize>().copied().unwrap_or_default().0;
    let view = canvas(physical);
    let scale = physical.y.max(1) as f32 / CANVAS_HEIGHT;
    let mut documents: Vec<(Entity, u32)> = Vec::new();
    Query::<&UiDocument>::new(resources)
        .for_each_entity(|entity, document| documents.push((entity, document.mode)));
    let (Some(mut fonts), Some(mut atlas)) = (
        resources.remove::<UiFonts>(),
        resources.remove::<GlyphAtlas>(),
    ) else {
        return;
    };
    if let Some(trees) = resources.get_mut::<UiTrees>() {
        lay_out(trees, &documents, view, scale, &mut fonts, &mut atlas);
    }
    resources.insert(fonts);
    resources.insert(atlas);
}

fn lay_out(
    trees: &mut UiTrees,
    documents: &[(Entity, u32)],
    view: glam::Vec2,
    scale: f32,
    fonts: &mut UiFonts,
    atlas: &mut GlyphAtlas,
) {
    for &(entity, mode) in documents {
        let Some(tree) = trees.get_mut(entity) else {
            continue;
        };
        let available = match mode {
            MODE_OVERLAY => Size {
                width: AvailableSpace::Definite(view.x),
                height: AvailableSpace::Definite(view.y),
            },
            _ => Size {
                width: AvailableSpace::MaxContent,
                height: AvailableSpace::MaxContent,
            },
        };
        tree.layout(available, fonts);
        tree.rasterize(scale, fonts, atlas);
    }
}

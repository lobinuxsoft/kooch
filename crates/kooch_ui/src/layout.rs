//! Laying every document out in the space its mode gives it.

use kooch_core::resource::Resources;
use kooch_ecs::query::Query;
use kooch_ecs::{Entity, ViewSize};
use taffy::{AvailableSpace, Size};

use crate::{MODE_OVERLAY, UiDocument, UiTrees};

/// Height of the logical canvas an Overlay is laid out in: the same UI fills the same share of a
/// handheld and a monitor, with no OS scale factor involved.
pub const CANVAS_HEIGHT: f32 = 1080.0;

/// The logical canvas for a view of `physical` pixels.
pub fn canvas(physical: glam::UVec2) -> glam::Vec2 {
    let height = physical.y.max(1) as f32;
    glam::Vec2::new(physical.x as f32 * CANVAS_HEIGHT / height, CANVAS_HEIGHT)
}

/// Overlay documents take the canvas; World ones their content size, until phase 5 sizes them.
pub fn layout_documents(resources: &mut Resources) {
    let view = canvas(resources.get::<ViewSize>().copied().unwrap_or_default().0);
    let mut documents: Vec<(Entity, u32)> = Vec::new();
    Query::<&UiDocument>::new(resources)
        .for_each_entity(|entity, document| documents.push((entity, document.mode)));
    let Some(trees) = resources.get_mut::<UiTrees>() else {
        return;
    };
    for (entity, mode) in documents {
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
        tree.layout(available);
    }
}

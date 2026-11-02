//! What the Library, the hierarchy and the Asset Browser drag into the document, and where it lands.

use kooch_core::Guid;
use kooch_ui::markup::Place;

use super::library::Item;
use crate::drag_drop::DraggedAsset;
use crate::panels::inspector::AssetCatalogEntry;
use crate::ui_builder::{SOURCE_TYPE, UiBuilder};

#[derive(Clone)]
pub(super) enum Dragged {
    Element(&'static Item),
    /// A `.kxml`, instanced with `<use>`.
    Template(Guid),
    /// A hierarchy row being moved.
    Row(u32),
}

/// What is being dragged, an Asset Browser `.kxml` included.
pub(super) fn current(ctx: &egui::Context) -> Option<Dragged> {
    if let Some(dragged) = egui::DragAndDrop::payload::<Dragged>(ctx) {
        return Some((*dragged).clone());
    }
    egui::DragAndDrop::payload::<DraggedAsset>(ctx)
        .filter(|asset| asset.type_name == SOURCE_TYPE)
        .map(|asset| Dragged::Template(asset.guid))
}

/// The dragged thing, taken, on the frame the button goes up.
pub(super) fn released(ctx: &egui::Context) -> Option<Dragged> {
    if !ctx.input(|input| input.pointer.any_released()) {
        return None;
    }
    let dragged = current(ctx)?;
    egui::DragAndDrop::clear_payload(ctx);
    Some(dragged)
}

pub(super) fn apply(
    builder: &mut UiBuilder,
    dragged: Dragged,
    at: Place,
    catalog: &[AssetCatalogEntry],
) {
    match dragged {
        Dragged::Element(item) => {
            builder.insert(item.snippet, at);
        }
        Dragged::Template(guid) => {
            // A file instancing itself never ends.
            let Some(entry) = catalog
                .iter()
                .find(|entry| entry.guid == guid && guid != builder.source)
            else {
                return;
            };
            let src = super::inspector::template_src(builder, &entry.path);
            if let Some(origin) = builder.insert("<use/>", at) {
                builder.set_attribute(origin, "src", &src);
            }
        }
        Dragged::Row(origin) => builder.move_to(origin, at),
    }
}

/// Into what holds children, beside what does not; the edges of a row put it beside either way.
pub(super) fn place_on(builder: &UiBuilder, origin: u32, fraction: f32) -> Place {
    if origin == 0 {
        return Place::Inside(0);
    }
    let inside = builder.holds_children(origin);
    match fraction {
        f if f < 0.25 => Place::Before(origin),
        f if f > 0.75 => Place::After(origin),
        _ if inside => Place::Inside(origin),
        f if f < 0.5 => Place::Before(origin),
        _ => Place::After(origin),
    }
}

/// A line where it lands beside `rect`, a frame where it lands inside.
pub(super) fn mark(painter: &egui::Painter, rect: egui::Rect, at: Place, color: egui::Color32) {
    let stroke = egui::Stroke::new(2.0, color);
    match at {
        Place::Inside(_) => {
            painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
        }
        Place::Before(_) => {
            painter.hline(rect.x_range(), rect.top(), stroke);
        }
        Place::After(_) => {
            painter.hline(rect.x_range(), rect.bottom(), stroke);
        }
    }
}

/// The tag riding the pointer while something is dragged.
pub(super) fn ghost(ctx: &egui::Context, builder: &UiBuilder) {
    let Some(dragged) = egui::DragAndDrop::payload::<Dragged>(ctx) else {
        return;
    };
    let Some(pos) = ctx.pointer_latest_pos() else {
        return;
    };
    let tag = match &*dragged {
        Dragged::Element(item) => item.tag,
        Dragged::Template(_) => "use",
        Dragged::Row(origin) => builder.tag(*origin).unwrap_or("?"),
    };
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("ui_builder_ghost"));
    ctx.layer_painter(layer).text(
        pos + egui::vec2(14.0, 4.0),
        egui::Align2::LEFT_TOP,
        format!("<{tag}>"),
        egui::FontId::monospace(12.0),
        ctx.global_style().visuals.strong_text_color(),
    );
}

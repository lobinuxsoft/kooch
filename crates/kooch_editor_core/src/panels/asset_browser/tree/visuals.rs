//! Icons and the drag preview — presentation with no state behind it.

use crate::icons;

pub(super) fn draw_drag_preview(ui: &egui::Ui, icon: &str, name: &str) {
    let Some(pos) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);

    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("asset_drag_preview"));
    let painter = ui.ctx().layer_painter(layer);
    let color = ui.visuals().strong_text_color();
    let galley = painter.layout_no_wrap(
        format!("{icon} {name}"),
        egui::FontId::proportional(13.0),
        color,
    );
    let text_pos = pos + egui::vec2(14.0, 8.0);
    let bg = egui::Rect::from_min_size(text_pos, galley.size()).expand(4.0);
    painter.rect_filled(bg, 3.0, ui.visuals().panel_fill);
    painter.galley(text_pos, galley, color);
}

/// Icon for a typed asset by its canonical type name.
pub(super) fn type_icon(type_name: &str) -> &'static str {
    match type_name {
        "kooch_render::meshlet::asset::MeshletMesh" => icons::CUBE,
        "kooch_render::material::asset::Material" => icons::FADERS,
        _ => icons::STACK,
    }
}

/// Icon for a plain (non-asset) file by extension.
pub(super) fn file_icon(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "rs" => icons::TERMINAL,
        "toml" | "lock" => icons::GEAR,
        "scene" => icons::TREE_STRUCTURE,
        "prefab" => icons::PACKAGE,
        _ => icons::LIST_BULLETS,
    }
}
